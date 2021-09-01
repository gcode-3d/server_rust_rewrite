pub mod models;
pub mod responses;
mod routes;
pub(crate) mod websocket_handler;

use crate::api_manager::responses::{
    not_found_response, server_error_response, unauthorized_response,
};

use self::{
    models::{AuthPermissions, EventInfo, StateWrapper},
    responses::bad_request_response,
};

use crossbeam_channel::Sender;
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
use hyper_tungstenite::WebSocketStream;
use sqlx::{Connection, SqliteConnection};
use std::{collections::HashMap, convert::Infallible, path::Path, sync::Arc};
use tokio::{spawn, sync::Mutex};

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
        sockets: Arc<Mutex<HashMap<u128, WebSocketStream<Upgraded>>>>,
        state: Arc<Mutex<StateWrapper>>,
    ) -> () {
        let file_server = Static::new(Path::new("client"));

        let make_svc = make_service_fn(move |_| {
            let distributor = distributor.clone();
            let state = state.clone();
            let sockets = sockets.clone();
            let file_server = file_server.clone();
            async move {
                Ok::<_, Error>(service_fn(move |req| {
                    let state = state.clone();
                    let dist_clone = distributor.clone();
                    let sockets = sockets.clone();
                    let file_server = file_server.clone();
                    async move { router(req, file_server, dist_clone, state, sockets).await }
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
    state: Arc<Mutex<StateWrapper>>,
    sockets: Arc<Mutex<HashMap<u128, WebSocketStream<Upgraded>>>>,
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
                    if let Err(e) = websocket_handler::handler(
                        websocket.await.expect("[WS] Handshake failure"),
                        user,
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
    let path = normalize_url(&request);
    if path.is_none() {
        return bad_request_response();
    }
    let path = path.unwrap();

    // In case the request is an OPTIONS request, handle with cors headers.
    if request.method() == Method::OPTIONS {
        return handle_option_requests(&request);
    }
    // Handle exact messages.
    if request.method() == Method::GET && path.eq(routes::ping::PATH) {
        return routes::ping::handler(request);
    }

    if request.method().eq(&Method::POST) && path.eq(routes::login::PATH) {
        return routes::login::handler(request).await;
    }

    if request.method().eq(&Method::GET) && path.eq(routes::dsn::PATH) {
        return routes::dsn::handler().await;
    }

    // From this point authed routes only
    let permissions = authenticate_route(&request).await;
    if permissions.is_none() {
        return unauthorized_response();
    };
    let permissions = permissions.unwrap();

    if request.method().eq(&Method::GET) && path.eq(routes::list_settings::PATH) {
        return routes::list_settings::handler().await;
    }

    if request.method().eq(&Method::POST) && path.eq(routes::update_settings::PATH) {
        if !permissions.settings_edit() {
            return unauthorized_response();
        }
        return routes::update_settings::handler(request).await;
    }

    if request.method().eq(&Method::GET) && path.eq(routes::list_files::PATH) {
        if !permissions.file_access() {
            return unauthorized_response();
        }
        return routes::list_files::handler(request).await;
    }

    if request.method().eq(&Method::POST) && path.eq(routes::upload_file::PATH) {
        if !permissions.file_edit() || !permissions.file_access() {
            return unauthorized_response();
        }
        return routes::upload_file::handler(&mut request).await;
    }

    if request.method().eq(&Method::PUT) && path.eq(routes::create_connection::PATH) {
        if !permissions.edit_connection() {
            return unauthorized_response();
        }
        return routes::create_connection::handler(
            request,
            distributor,
            state.lock().await.clone(),
        )
        .await;
    }

    if request.method().eq(&Method::DELETE) && path.eq(routes::disconnect_connection::PATH) {
        if !permissions.edit_connection() {
            return unauthorized_response();
        }
        let state = state.lock().await.state.clone();
        return routes::disconnect_connection::handler(state, distributor).await;
    }

    if request.method().eq(&Method::POST) && path.eq(routes::reconnect_connection::PATH) {
        if !permissions.edit_connection() {
            return unauthorized_response();
        }
        let state = state.lock().await.state.clone();
        return routes::reconnect_connection::handler(state, distributor).await;
    }

    if request.method().eq(&Method::PUT) && path.eq(routes::start_print::PATH) {
        if !permissions.print_state_edit() {
            return unauthorized_response();
        }
        return routes::start_print::handler(request, distributor, state).await;
    }

    if request.method().eq(&Method::DELETE) && path.eq(routes::cancel_print::PATH) {
        if !permissions.print_state_edit() {
            return unauthorized_response();
        }
        return routes::cancel_print::handler(state.lock().await.clone(), distributor);
    }

    if request.method().eq(&Method::POST) && path.eq(routes::terminal::PATH) {
        if !permissions.terminal_send() {
            return unauthorized_response();
        }
        let state = state.lock().await.state.clone();
        return routes::terminal::handler(request, distributor, state).await;
    }

    return not_found_response();
}

async fn authenticate_route(request: &Request<Body>) -> Option<AuthPermissions> {
    if !request.headers().contains_key("authorization") {
        return None;
    }

    let token = request
        .headers()
        .get("authorization")
        .unwrap()
        .to_str()
        .expect("Not a valid value");

    if token.len() != 60 || !token.chars().all(char::is_alphanumeric) {
        return None;
    }
    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    let mut query = sqlx::query_as::<_, AuthPermissions>(
                "select a.username as username, a.permissions as permissions from users a inner join tokens b on a.username = b.username where (b.expire < DATE('now') OR b.expire is null) AND b.token = ?",
            );

    query = query.bind(token);

    let result = query.fetch_one(&mut connection).await;

    if result.is_err() {
        return None;
    }
    return Some(result.unwrap());
}

/*
    Function gets called by the handle_route function.

    Based on the path, create a new response and add the appropriate corse headers.

    Arguments:
    - request: Original hyper request.

*/
fn handle_option_requests(request: &Request<Body>) -> Response<Body> {
    let path = normalize_url(&request);
    if path.is_none() {
        return bad_request_response();
    }
    let path = path.unwrap();
    if path == routes::login::PATH {
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, routes::login::METHODS)
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
            .body(Body::empty())
            .expect("Couldn't create a valid response");
    }
    if path == routes::ping::PATH {
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, routes::ping::METHODS)
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "")
            .body(Body::empty())
            .expect("Couldn't create a valid response");
    }
    if path == routes::list_settings::PATH {
        return Response::builder()
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
            .expect("Couldn't create a valid response");
    }

    if path == routes::upload_file::PATH {
        return Response::builder()
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
            .expect("Couldn't create a valid response");
    }

    if path == routes::create_connection::PATH {
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                routes::create_connection::METHODS,
            )
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
            .body(Body::empty())
            .expect("Couldn't create a valid response");
    }
    if path == routes::start_print::PATH {
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                routes::start_print::METHODS,
            )
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
            .body(Body::empty())
            .expect("Couldn't create a valid response");
    }

    if path == routes::dsn::PATH {
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, routes::dsn::METHODS)
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
            .body(Body::empty())
            .expect("Couldn't create a valid response");
    }
    if path == routes::terminal::PATH {
        return Response::builder()
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(
                header::ACCESS_CONTROL_ALLOW_METHODS,
                routes::terminal::METHODS,
            )
            .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "*")
            .body(Body::empty())
            .expect("Couldn't create a valid response");
    }
    return not_found_response();
}

fn normalize_url(request: &Request<Body>) -> Option<String> {
    let mut path = request.uri().path().to_string();
    if !path.chars().all(|x| x.is_ascii()) {
        return None;
    }
    if path.len() > 1 {
        if path.clone().chars().last().unwrap() == '/' {
            let mut chars = path.chars();
            chars.next_back();
            path = chars.as_str().to_string();
        }
    }
    return Some(path);
}
