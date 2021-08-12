pub mod models;
pub mod responses;
mod routes;

use crate::{
    api_manager::{
        models::EventType,
        responses::{not_found_response, server_error_response, unauthorized_response},
    },
    bridge::BridgeState,
};

use self::{
    models::{AuthPermissions, EventInfo, StateWrapper},
    responses::bad_request_response,
};

use chrono::{DateTime, Utc};
use crossbeam_channel::{unbounded, Receiver, Sender};
use futures::{sink::SinkExt, stream::StreamExt, FutureExt};
use hyper::{
    header::{self, HeaderValue, ACCESS_CONTROL_ALLOW_ORIGIN},
    upgrade::Upgraded,
    Error,
};
use hyper::{
    service::{make_service_fn, service_fn},
    Method,
};
use hyper::{Body, Request, Response, Server};
use hyper_staticfile::Static;
use hyper_tungstenite::tungstenite::Message;
use hyper_tungstenite::{
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame},
    WebSocketStream,
};
use serde_json::{json, Value};
use sqlx::{Connection, SqliteConnection};
use std::{collections::HashMap, convert::Infallible, path::Path, sync::Arc};
use tokio::{spawn, sync::Mutex, task::yield_now};
use uuid::Uuid;

pub struct ApiManager {}

/*
    Function that starts a hyper server.

    Creates a route handler.
    Create the socket hashmap, and setup arcs for state.

    Arguments:
    - distributor: The sender for the global events channel.
    - distributor_receiver: The receiver for the global events channel, intended for the websocket connections.


*/
impl ApiManager {
    pub async fn start(
        distributor: Sender<EventInfo>,
        distributor_receiver: Receiver<EventInfo>,
    ) -> () {
        let sockets: Arc<Mutex<HashMap<u128, Sender<String>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        let state: Arc<Mutex<StateWrapper>> = Arc::new(Mutex::new(StateWrapper {
            state: BridgeState::DISCONNECTED,
            description: models::StateDescription::None,
        }));
        let file_server = Static::new(Path::new("client"));

        let make_svc = make_service_fn(move |_| {
            let distributor = distributor.clone();
            let receiver = distributor_receiver.clone();
            let state = state.clone();
            let sockets = sockets.clone();
            let file_server = file_server.clone();
            async move {
                Ok::<_, Error>(service_fn(move |req| {
                    let state = state.clone();
                    let dist_clone = distributor.clone();
                    let rcvr_clone = receiver.clone();
                    let sockets = sockets.clone();
                    let file_server = file_server.clone();
                    async move { router(req, file_server, dist_clone, rcvr_clone, state, sockets).await }
                }))
            }
        });

        let addr = ([0, 0, 0, 0], 8000).into();

        let server = Server::bind(&addr).serve(make_svc);
        println!("[API] Listening on http://{}", addr);
        let _ = server.await;
    }
}
/*
    Function that gets called when a new request is received.
    Based on path the path is routed using a route function,
    or upgraded and passed along to the websocket handler.

    Arguments:
    - req: The hyper request.
    - distributor: The sender for the global events channel.
    - receiver: The receiver for the global events channel.
    - state: current state arc, used by websockets.
    - sockets: hashmap including all websocket senders, mapped by uuid.

*/
async fn router(
    mut req: Request<Body>,
    file_server: Static,
    distributor: Sender<EventInfo>,
    receiver: Receiver<EventInfo>,
    state: Arc<Mutex<StateWrapper>>,
    sockets: Arc<Mutex<HashMap<u128, Sender<String>>>>,
) -> Result<Response<Body>, Infallible> {
    /*
    In case the request is an upgrade request, and the path is /ws:
    Check if the request has the correct headers and tokens and start upgrading.
    Pass the upgraded connection to the websocket_handler.
    */
    if hyper_tungstenite::is_upgrade_request(&req) && req.uri().path().eq("/ws") {
        if !req.headers().contains_key("sec-websocket-protocol") {
            return Ok(unauthorized_response());
        }

        let token = String::from(
            req.headers()
                .clone()
                .get("sec-websocket-protocol")
                .unwrap()
                .to_str()
                .unwrap(),
        );
        if token.contains(" ")
            || token.contains(",")
            || token.len() != 60
            || !token.chars().all(char::is_alphanumeric)
        {
            return Ok(unauthorized_response());
        }

        let result = async {
            let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
            let mut query = sqlx::query_as::<_, AuthPermissions>(
                "select a.username as username, a.permissions as permissions from users a inner join tokens b on a.username = b.username where (b.expire < DATE('now') OR b.expire is null) AND b.token = ?",
            );

            query = query.bind(&token);

            match query.fetch_optional(&mut connection).await {
                Ok(value) => {
                    if value.is_none() {
                        return None;
                    }else {
                        return Some(value.unwrap());
                    }
                },
                Err(err) => {
                    eprintln!("[WS][ERROR] {}", err);
                    return None;
                },
            }
        }
        .await;

        if result.is_none() {
            return Ok(unauthorized_response());
        }
        let user = result.unwrap();

        match hyper_tungstenite::upgrade(req, None) {
            Ok((mut response, websocket)) => {
                spawn(async move {
                    if let Err(e) = websocket_handler(
                        websocket.await.expect("[WS] Handshake failure"),
                        user,
                        receiver,
                        state,
                        sockets,
                    )
                    .await
                    {
                        eprintln!("Error websocket: {}", e);
                    }
                });
                /*
                Although not all browers expect/support it,
                even although we are using the protocol headers incorrectly,
                we still comply with the standard by sending back the same "protocol" (token).
                */
                response.headers_mut().append(
                    header::SEC_WEBSOCKET_PROTOCOL,
                    HeaderValue::from_str(&token).unwrap(),
                );
                return Ok(response);
            }
            Err(e) => {
                eprintln!("Error upgrading: {}", e);
                return Ok(Response::builder()
                    .body(Body::from("Internal Server Error"))
                    .expect("Failed to construct a valid response"));
            }
        }
    } else if req.uri().path().eq("/ws") {
        return Ok(bad_request_response());
    } else if req.uri().path().starts_with("/api/") {
        return Ok(handle_route(req, distributor, state).await);
    } else {
        if !req.uri().path().contains(".") {
            *req.uri_mut() = "/".parse().unwrap();
        }
        return match file_server.serve(req).await {
            Ok(mut response) => {
                response.headers_mut().append(
                    ACCESS_CONTROL_ALLOW_ORIGIN,
                    HeaderValue::from_str("*").unwrap(),
                );
                Ok(response)
            }
            Err(err) => {
                eprintln!("[FILE_SERVER][ERROR] {}", err);
                Ok(server_error_response())
            }
        };
    }
}

/*
    Function gets called by the router after the request has been upgraded to a websocket connection.
    The function keeps loaded as long as a connection is created


    Arguments:
    - websocket: The websocket object
    - user: parsed user including permissions.
    - receiver: Global receiver to catch events related to websockets.
    - state: current state arc, used for sending intial ready event.
    - sockets: hashmap including all websocket senders, mapped by uuid.

*/
async fn websocket_handler(
    websocket: WebSocketStream<Upgraded>,
    user: AuthPermissions,
    receiver: Receiver<EventInfo>,
    state: Arc<Mutex<StateWrapper>>,
    sockets: Arc<Mutex<HashMap<u128, Sender<String>>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (outgoing, mut incoming) = websocket.split();
    let outgoing = Arc::new(Mutex::new(outgoing));
    let (local_sender, local_receiver) = unbounded::<String>();
    let id = Uuid::new_v4();
    {
        sockets
            .lock()
            .await
            .insert(id.clone().as_u128(), local_sender);

        println!(
            "[WS] New connection: {} | User: {}",
            id.to_hyphenated(),
            &user.username()
        );
    }
    let current_state = state.clone();
    let sockets_clone = sockets.clone();
    /*
        Setup a receiver reader for the global event channel.
    */
    spawn(async move {
        loop {
            if let Ok(event) = receiver.try_recv() {
                match event.event_type {
                    models::EventType::Websocket(models::WebsocketEvents::TempUpdate {
                        tools,
                        bed,
                        chamber,
                    }) => {
                        let json = json!({
                            "type": "temperature_change",
                            "content": {
                                "tools": tools,
                                "bed": bed,
                                "chamber": chamber,
                                "time": Utc::now().timestamp_millis()
                            },
                        });
                        for sender in sockets_clone.lock().await.iter() {
                            sender
                                .1
                                .send(json.to_string())
                                .expect("Cannot send message");
                        }
                    }
                    models::EventType::Websocket(models::WebsocketEvents::TerminalRead {
                        message,
                    }) => {
                        let time: DateTime<Utc> = Utc::now();
                        let json = json!({
                            "type": "terminal_message",
                            "content": [
                                {
                                    "message": message,
                                    "type": "OUTPUT",
                                    "id": null,
                                    "time": time.to_rfc3339()
                                }
                            ]
                        });
                        for sender in sockets_clone.lock().await.iter() {
                            let result = sender.1.send(json.to_string());
                            if result.is_err() {
                                println!(
                                    "[WS] Connection closed: {}",
                                    Uuid::from_u128(sender.0.clone()).to_hyphenated()
                                );
                                sockets_clone.lock().await.remove(sender.0);
                            }
                        }
                    }
                    models::EventType::Websocket(models::WebsocketEvents::TerminalSend {
                        message,
                    }) => {
                        let time: DateTime<Utc> = Utc::now();
                        let json = json!({
                            "type": "terminal_message",
                            "content": [
                                {
                                    "message": message,
                                    "type": "INPUT",
                                    "id": null,
                                    "time": time.to_rfc3339()
                                }
                            ]
                        });
                        for sender in sockets_clone.lock().await.iter() {
                            sender
                                .1
                                .send(json.to_string())
                                .expect("Cannot send message");
                        }
                    }

                    models::EventType::Websocket(models::WebsocketEvents::StateUpdate {
                        state,
                        description,
                    }) => {
                        *current_state.lock().await = StateWrapper {
                            state,
                            description: description.clone(),
                        };
                        let json = match state {
                            BridgeState::DISCONNECTED => json!({
                                "type": "state_update",
                                "content": {
                                    "state": "Disconnected",
                                    "description": serde_json::Value::Null
                                }
                            })
                            .to_string(),
                            BridgeState::CONNECTING => json!({
                                "type": "state_update",
                                "content": {
                                    "state": "Connecting",
                                    "description": serde_json::Value::Null
                                }
                            })
                            .to_string(),
                            BridgeState::CONNECTED => json!({
                                "type": "state_update",
                                "content": {
                                    "state": "Connected",
                                    "description": serde_json::Value::Null
                                }
                            })
                            .to_string(),
                            BridgeState::ERRORED => match description {
                                models::StateDescription::Error { message } => json!({
                                    "type": "state_update",
                                    "content": {
                                        "state": "Errored",
                                        "description": {
                                            "errorDescription": message
                                        }
                                    }
                                })
                                .to_string(),
                                _ => json!({
                                    "type": "state_update",
                                    "content": {
                                        "state": "Errored",
                                        "description": serde_json::Value::Null
                                    }
                                })
                                .to_string(),
                            },
                            BridgeState::PREPARING => todo!(),
                            BridgeState::PRINTING => match description {
                                models::StateDescription::Print {
                                    filename,
                                    progress,
                                    start,
                                    end,
                                } => {
                                    let mut end_string: Option<String> = None;
                                    if end.is_some() {
                                        end_string = Some(end.unwrap().to_rfc3339());
                                    }
                                    json!({
                                        "type": "state_update",
                                        "content": {
                                            "state": "Printing",
                                            "description": {
                                                "printInfo": {
                                                    "file": {
                                                        "name": filename,
                                                    },
                                                    "progress": format!("{:.2}", progress),
                                                    "startTime": start.to_rfc3339(),
                                                    "estEndTime": end_string
                                                }
                                            }
                                        }
                                    })
                                    .to_string()
                                }
                                _ => json!({
                                    "type": "state_update",
                                    "content": {
                                        "state": "Printing",
                                        "description": serde_json::Value::Null
                                    }
                                })
                                .to_string(),
                            },
                            BridgeState::FINISHING => todo!(),
                        };
                        for sender in sockets_clone.lock().await.iter() {
                            sender
                                .1
                                .send(json.to_string())
                                .expect("Cannot send message");
                        }
                    }
                    EventType::Bridge(_) => todo!(),
                    EventType::KILL => (),
                }
            } else {
                yield_now().await;
            }
        }
    });
    let outgoing_clone = outgoing.clone();
    let sockets_clone = sockets.clone();
    /*
        Read incoming messages from the websocket connection.
    */
    spawn(async move {
        loop {
            if let Some(result) = incoming.next().now_or_never() {
                if result.is_none() {
                    yield_now().await;
                    continue;
                }
                let result = result.unwrap();

                match result {
                    Ok(message) => {
                        if message.is_close() {
                            sockets_clone
                                .lock()
                                .await
                                .remove(&id.as_u128())
                                .expect("Cannot remove socket");
                            println!("[WS] Connection closed: {}", &id.to_hyphenated());

                            let _ = outgoing_clone
                                .lock()
                                .await
                                .send(Message::Close(Some(CloseFrame {
                                    code: CloseCode::Normal,
                                    reason: std::borrow::Cow::Borrowed(""),
                                })))
                                .await;

                            continue;
                        }
                        if message.is_ping() {
                            outgoing_clone
                                .lock()
                                .await
                                .send(Message::Pong(message.into_data()))
                                .await
                                .expect("Cannot send message");
                            continue;
                        }
                        if message.is_text() == false {
                            println!("[WS] Connection closed: {}", &id.to_hyphenated());
                            let _ = sockets_clone.lock().await.remove(&id.as_u128());
                            outgoing_clone
                                .lock()
                                .await
                                .send(Message::Close(Some(CloseFrame {
                                    code: CloseCode::Policy,
                                    reason: std::borrow::Cow::Borrowed("Bad data"),
                                })))
                                .await
                                .expect("Cannot send message");
                            continue;
                        }

                        outgoing_clone
                            .lock()
                            .await
                            .send(Message::text("Boop back!"))
                            .await
                            .expect("Cannot send message");
                    }
                    Err(e) => {
                        eprintln!("[WS][ERROR] {}", e);
                        sockets_clone.lock().await.remove(&id.as_u128());
                    }
                }
            } else {
                yield_now().await;
            }
        }
    });
    let outgoing_clone = outgoing.clone();

    /*
        Read messages coming from the local_sender pushed into the hashmap.
    */
    spawn(async move {
        loop {
            if let Ok(message) = local_receiver.try_recv() {
                outgoing_clone
                    .lock()
                    .await
                    .send(Message::text(message))
                    .await
                    .expect("Cannot send message");
            } else {
                yield_now().await;
            }
        }
    });

    /*
        Construct intial ready event message.
    */
    let mut json = json!({
            "type":"ready",
            "content": {}
    });
    let content = json.get_mut("content").unwrap();
    let user = json!({
    "username": user.username(),
    "permissions" : {
         "admin": user.admin() ,
         "connection.edit": user.edit_connection(),
         "file.access": user.file_access(),
         "file.edit": user.file_edit(),
         "print_state.edit": user.print_state_edit(),
         "settings.edit": user.settings_edit(),
         "permissions.edit": user.users_edit(),
         "terminal.read": user.terminal_read(),
         "terminal.send": user.terminal_send(),
         "webcam.view": user.webcam(),
         "update.check": user.update(),
         "update.manage": user.update()
        }
    });

    let state_info = state.lock().await;
    match state_info.state {
        BridgeState::DISCONNECTED => {
            *content = json!({
                "user": user,
                "state": "Disconnected",
            });
        }
        BridgeState::CONNECTING => {
            *content = json!({
                "user": user,
                "state": "Connecting",
            });
        }
        BridgeState::CONNECTED => {
            *content = json!({
                "user": user,
                "state": "Connected",
            });
        }
        BridgeState::ERRORED => {
            let description = match state_info.description.clone() {
                models::StateDescription::Error { message } => message,
                _ => "Unknown error".to_string(),
            };

            *content = json!({
                "user": user,
                "state": "Errored",
                "description": {
                    "errorDescription": description
                }
            });
        }
        BridgeState::PREPARING => todo!(),
        BridgeState::PRINTING => {
            let description = match state_info.description.clone() {
                models::StateDescription::Print {
                    filename,
                    progress,
                    start,
                    end,
                } => {
                    let mut end_string = None;
                    if end.is_some() {
                        end_string = Some(end.unwrap().to_rfc3339());
                    }
                    json!({
                        "printInfo": {
                            "file": {
                                "name": filename,
                            },
                        "progress": format!("{:.2}", progress),
                        "startTime": start.to_rfc3339(),
                        "estEndTime": end_string
                    }})
                }
                _ => Value::Null,
            };
            *content = json!({
                "user": user,
                "state": "Printing",
                "description": description
            });
        }
        BridgeState::FINISHING => todo!(),
    };

    outgoing
        .lock()
        .await
        .send(Message::text(json.to_string()))
        .await
        .expect("Cannot send message");
    return Ok(());
}

/*
    Function gets called by the router after the request isn't a websocket upgrade request.

    Arguments:
    - request: Original hyper request.
    - distributor: Global sender to send events to.

*/
async fn handle_route(
    mut request: Request<Body>,
    distributor: Sender<EventInfo>,
    state: Arc<Mutex<StateWrapper>>,
) -> Response<Body> {
    // In case the request is an OPTIONS request, handle with cors headers.
    if request.method() == Method::OPTIONS {
        let result = handle_option_requests(&request);
        if result.is_some() {
            return result.unwrap();
        }
    }

    // Handle exact messages.
    if request.method() == Method::GET && request.uri().path().eq(routes::ping::PATH) {
        return routes::ping::handler(request);
    }
    if request.method().eq(&Method::POST) && request.uri().path().eq(routes::login::PATH) {
        return routes::login::handler(request).await;
    }
    if request.method().eq(&Method::GET) && request.uri().path().eq(routes::list_settings::PATH) {
        return routes::list_settings::handler(request).await;
    }
    if request.method().eq(&Method::POST) && request.uri().path().eq(routes::update_settings::PATH)
    {
        return routes::update_settings::handler(request).await;
    }
    if request.method().eq(&Method::GET) && request.uri().path().eq(routes::list_files::PATH) {
        return routes::list_files::handler(request).await;
    }
    if request.method().eq(&Method::PUT) && request.uri().path().eq(routes::upload_file::PATH) {
        return routes::upload_file::handler(&mut request).await;
    }
    if request.method().eq(&Method::PUT) && request.uri().path().eq(routes::create_connection::PATH)
    {
        return routes::create_connection::handler(request, distributor).await;
    }
    if request.method().eq(&Method::PUT) && request.uri().path().eq(routes::start_print::PATH) {
        return routes::start_print::handler(request, distributor, state).await;
    }
    if request.method().eq(&Method::GET) && request.uri().path().eq(routes::dsn::PATH) {
        return routes::dsn::handler().await;
    }
    return not_found_response();
}

/*
    Function gets called by the handle_route function.

    Based on the path, create a new response and add the appropriate corse headers.

    Arguments:
    - request: Original hyper request.

*/
fn handle_option_requests(request: &Request<Body>) -> Option<Response<Body>> {
    if request.uri().path() == routes::login::PATH {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::ACCESS_CONTROL_ALLOW_METHODS, routes::login::METHODS)
                .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    if request.uri().path() == routes::ping::PATH {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::ACCESS_CONTROL_ALLOW_METHODS, routes::ping::METHODS)
                .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    if request.uri().path() == routes::list_settings::PATH {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    routes::list_settings::METHODS,
                )
                .header(
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    "Authorization, Content-Type",
                )
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }

    if request.uri().path() == routes::upload_file::PATH {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    routes::upload_file::METHODS,
                )
                .header(
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    "X-Requested-With,content-type, Authorization, X-force-upload",
                )
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }

    if request.uri().path() == routes::create_connection::PATH {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    routes::create_connection::METHODS,
                )
                .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    if request.uri().path() == routes::start_print::PATH {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(
                    header::ACCESS_CONTROL_ALLOW_METHODS,
                    routes::start_print::METHODS,
                )
                .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }

    if request.uri().path() == routes::dsn::PATH {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::ACCESS_CONTROL_ALLOW_METHODS, routes::dsn::METHODS)
                .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    return None;
}
