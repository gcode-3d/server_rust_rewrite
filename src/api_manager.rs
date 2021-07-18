pub mod models;
pub mod responses;

mod routes;

use crate::api_manager::{
    models::{BridgeEvents, EventType, State},
    responses::{not_found_response, unauthorized_response},
};

use self::{
    models::{AuthPermissions, EventInfo},
    responses::bad_request_response,
    routes::ping,
};

use chrono::{DateTime, Utc};
use crossbeam_channel::{unbounded, Receiver, Sender};
use hyper::{
    header::{self, HeaderValue},
    upgrade::Upgraded,
    Error,
};

use hyper::{
    service::{make_service_fn, service_fn},
    Method,
};
use hyper_tungstenite::{
    tungstenite::protocol::{frame::coding::CloseCode, CloseFrame},
    WebSocketStream,
};

use futures::{sink::SinkExt, stream::StreamExt};
use hyper::{Body, Request, Response, Server};
use hyper_tungstenite::tungstenite::Message;
use serde_json::json;
use sqlx::{Connection, SqliteConnection};
use std::{convert::Infallible, ops::Deref, sync::Arc};
use tokio::{spawn, sync::Mutex};
pub struct ApiManager {}

impl ApiManager {
    pub async fn start(
        distributor: Sender<EventInfo>,
        websocket_receiver: Receiver<EventInfo>,
    ) -> () {
        let make_svc = make_service_fn(move |_| {
            let distributor = distributor.clone();
            let receiver = websocket_receiver.clone();
            let state: Arc<Mutex<State>> = Arc::new(Mutex::new(State::Disconnected));
            async move {
                Ok::<_, Error>(service_fn(move |req| {
                    let curr_state = state.clone();
                    let dist_clone = distributor.clone();
                    let rcvr_clone = receiver.clone();
                    async move { basic_handler(req, dist_clone, rcvr_clone, curr_state).await }
                }))
            }
        });

        let addr = ([0, 0, 0, 0], 8000).into();

        let server = Server::bind(&addr).serve(make_svc);
        println!("Listening on http://{}", addr);
        let _ = server.await;
    }
}

async fn basic_handler(
    req: Request<Body>,
    distributor: Sender<EventInfo>,
    receiver: Receiver<EventInfo>,
    state: Arc<Mutex<State>>,
) -> Result<Response<Body>, Infallible> {
    if hyper_tungstenite::is_upgrade_request(&req) && req.uri().path().eq("/ws") {
        println!("starting ws upgrade");
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
                    )
                    .await
                    {
                        eprintln!("Error websocket: {}", e);
                    }
                });
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
    } else {
        return Ok(handle_route(req, distributor).await);
    }
}

async fn websocket_handler(
    websocket: WebSocketStream<Upgraded>,
    user: AuthPermissions,
    receiver: Receiver<EventInfo>,
    state: Arc<Mutex<State>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let (outgoing, mut incoming) = websocket.split();
    let outgoing = Arc::new(Mutex::new(outgoing));
    println!("{:?}", user);
    let (local_sender, local_receiver) = unbounded::<String>();
    let current_state = state.clone();
    spawn(async move {
        while let Some(event) = receiver.iter().next() {
            match event.event_type {
                models::EventType::Websocket(models::WebsocketEvents::TerminalSend {
                    message: _,
                }) => {
                    todo!("Not yet impl. terminal send");
                }

                models::EventType::Websocket(models::WebsocketEvents::TerminalRead { message }) => {
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
                    let _ = local_sender.send(json.to_string());
                }

                models::EventType::Websocket(models::WebsocketEvents::StateUpdate { state }) => {
                    *current_state.lock().await = state.clone();
                    let json = match state {
                        models::State::Disconnected => json!({
                            "type": "state_update",
                            "content": {
                                "state": "Disconnected",
                                "description": null
                            }
                        })
                        .to_string(),
                        models::State::Connecting => json!({
                            "type": "state_update",
                            "content": {
                                "state": "Connecting",
                                "description": null
                            }
                        })
                        .to_string(),
                        models::State::Connected => json!({
                            "type": "state_update",
                            "content": {
                                "state": "Connected",
                                "description": null
                            }
                        })
                        .to_string(),
                        models::State::Errored { description } => json!({
                            "type": "state_update",
                            "content": {
                                "state": "Errored",
                                "description": description
                            }
                        })
                        .to_string(),
                    };
                    let _ = local_sender.send(json);
                }
                EventType::Bridge(_) => todo!(),
                EventType::Websocket(_) => todo!(),
            }
        }
    });
    let outgoing_clone = outgoing.clone();
    spawn(async move {
        while let Some(result) = incoming.next().await {
            match result {
                Ok(message) => {
                    println!("Incoming: {:?}", message);

                    if message.is_close() {
                        let _ = outgoing_clone
                            .lock()
                            .await
                            .send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Normal,
                                reason: std::borrow::Cow::Borrowed(""),
                            })))
                            .await;
                    }
                    if message.is_text() == false {
                        let _ = outgoing_clone
                            .lock()
                            .await
                            .send(Message::Close(Some(CloseFrame {
                                code: CloseCode::Policy,
                                reason: std::borrow::Cow::Borrowed("Bad data"),
                            })))
                            .await;
                    }

                    let _ = outgoing_clone
                        .lock()
                        .await
                        .send(Message::text("Boop back!"))
                        .await;
                }
                Err(e) => {
                    eprintln!("{}", e);
                }
            }
        }
    });
    let outgoing_clone = outgoing.clone();
    spawn(async move {
        while let Some(message) = local_receiver.iter().next() {
            let _ = outgoing_clone
                .lock()
                .await
                .send(Message::text(message))
                .await;
        }
    });
    let mut json = json!({
            "type":"ready",
            "content": {
                "user":
                {
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
                    }
            }
    })
    .to_string();
    json.chars().next_back();
    json.chars().next_back();

    match state.lock().await.deref() {
        State::Disconnected => {
            json = format!("{}, {{\"state\": \"Disconnected\"}}}}}}", json);
        }
        State::Connecting => {
            json = format!("{}, {{\"state\": \"Connecting\"}}}}}}", json);
        }
        State::Connected => {
            json = format!("{}, {{\"state\": \"Connected\"}}}}}}", json);
        }
        State::Errored { description } => {
            json = format!(
                "{}, {}}}}}",
                json,
                json!({
                    "state": "errored",
                    "description": description
                })
            )
        }
    };
    let _ = outgoing.lock().await.send(Message::text(json)).await;
    return Ok(());
}

async fn handle_route(
    mut request: Request<Body>,
    distributor: Sender<EventInfo>,
) -> Response<Body> {
    println!("[API] Request url: {}", request.uri());

    if !request.uri().path().starts_with("/api") {
        println!("Matched first fallback, no /api");
        return not_found_response();
    }

    if request.method() == Method::OPTIONS {
        let result = handle_option_requests(&request);
        if result.is_some() {
            return result.unwrap();
        }
    }

    // First do exact matches
    if request.method() == Method::GET && request.uri().path().eq("/api/ping") {
        let _ = distributor.send(EventInfo {
            event_type: EventType::Bridge(BridgeEvents::ConnectionCreate {
                address: "com3".to_string(),
                port: 115200,
            }),
            message_data: "".to_string(),
        });
        return ping::handler(request);
    }
    if request.method().eq(&Method::POST) && request.uri().path().eq("/api/login") {
        return routes::login::handler(request).await;
    }
    if request.method().eq(&Method::GET) && request.uri().path().eq("/api/settings") {
        return routes::list_settings::handler(request).await;
    }
    if request.method().eq(&Method::GET) && request.uri().path().eq("/api/files") {
        return routes::list_files::handler(request).await;
    }
    if request.method().eq(&Method::PUT) && request.uri().path().eq("/api/files") {
        return routes::upload_file::handler(&mut request).await;
    }
    return not_found_response();
}

fn handle_option_requests(request: &Request<Body>) -> Option<Response<Body>> {
    if request.uri().path() == "/api/login" {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::ACCESS_CONTROL_ALLOW_METHODS, "POST")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    if request.uri().path() == "/api/ping" {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    if request.uri().path() == "/api/settings" {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST")
                .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Authorization")
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    if request.uri().path() == "/api/files" {
        return Some(
            Response::builder()
                .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
                .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, PUT")
                .header(
                    header::ACCESS_CONTROL_ALLOW_HEADERS,
                    "X-Requested-With,content-type, Authorization, X-force-upload",
                )
                .body(Body::empty())
                .expect("Couldn't create a valid response"),
        );
    }
    return None;
}
