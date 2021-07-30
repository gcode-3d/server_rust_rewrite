use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::Path,
    sync::Arc,
};

use chrono::Utc;
use crossbeam_channel::Sender;
use hyper::{body, header, Body, Request, Response};
use serde::Deserialize;
use serde_json::Value;
use sqlx::{Connection, SqliteConnection};
use tokio::sync::Mutex;

use crate::{
    api_manager::{
        models::{AuthPermissions, BridgeEvents, EventInfo, EventType, PrintInfo, StateWrapper},
        responses::{
            bad_request_response, forbidden_response, not_found_response, server_error_response,
            unauthorized_response,
        },
    },
    bridge::BridgeState,
};

pub const PATH: &str = "/api/print";
pub const METHODS: &str = "PUT";

pub async fn handler(
    mut req: Request<Body>,
    distributor: Sender<EventInfo>,
    state: Arc<Mutex<StateWrapper>>,
) -> Response<Body> {
    let headers = req.headers().clone();

    if !headers.contains_key("authorization") {
        return unauthorized_response();
    }
    let token = headers
        .get("authorization")
        .unwrap()
        .to_str()
        .expect("Not a valid value");

    if token.len() != 60 || !token.chars().all(char::is_alphanumeric) {
        return unauthorized_response();
    }

    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    let mut query = sqlx::query_as::<_, AuthPermissions>(
									"select a.username as username, a.permissions as permissions from users a inner join tokens b on a.username = b.username where (b.expire < DATE('now') OR b.expire is null) AND b.token = ?",
							);

    query = query.bind(token);

    let result = query.fetch_one(&mut connection).await;
    if result.is_err() {
        eprintln!("[API][PRINT] Error: {}", result.unwrap_err());
        return bad_request_response();
    }
    let permissions = result.unwrap();

    if !permissions.print_state_edit() {
        return unauthorized_response();
    }

    let result = body::to_bytes(req.body_mut()).await.unwrap();
    let body = match String::from_utf8(result.to_vec()) {
        Ok(body) => Some(body),
        Err(e) => {
            eprintln!("[API][upd. set] Invalid body received: {}", e);
            None
        }
    };
    if body.is_none() {
        return bad_request_response();
    }

    let json = serde_json::from_str::<Value>(&body.unwrap());

    if json.is_err() {
        return bad_request_response();
    }
    let json = json.unwrap();
    let filename = json.get("printName");
    if filename.is_none() {
        return bad_request_response();
    }
    let filename = filename.unwrap().as_str();
    if filename.is_none() {
        return bad_request_response();
    }
    let filename = filename.unwrap().trim();
    if filename.is_empty() || !filename.ends_with(".gcode") {
        return bad_request_response();
    }
    println!("[STARTPRINT] [STATE]: {:?}", state.lock().await);
    if state.lock().await.state.ne(&BridgeState::CONNECTED) {
        return forbidden_response();
    }

    let path = Path::new("./files/")
        .join(filename)
        .to_string_lossy()
        .into_owned();

    let file = File::open(path);

    if file.is_err() {
        return not_found_response();
    }
    let file = file.unwrap();
    let meta = file.metadata();
    if meta.is_err() {
        return server_error_response();
    }
    let size = meta.unwrap().len();

    let reader = BufReader::new(file);
    let file_reader = reader.lines();
    distributor
        .send(EventInfo {
            event_type: EventType::Bridge(BridgeEvents::PrintStart {
                info: PrintInfo::new(filename.to_string(), size, Some(file_reader), Utc::now()),
            }),
        })
        .expect("Couldn't send message");

    return Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Authorization, Content-Type",
        )
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .status(201)
        .body(Body::empty())
        .expect("Failed to construct valid response");
}

#[derive(Deserialize, Debug)]
#[allow(non_snake_case)]
struct JsonSettingRow {
    pub settingName: String,
    pub settingValue: String,
}
