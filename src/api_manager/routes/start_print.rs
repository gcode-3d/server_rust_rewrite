/*
    Reads a file from the files folder. Load it into memory.
    Constructs a print info file and start a print

    PUT /api/print

    Body: (json)
        printName: String


    Permission: print_state.edit
    State: Connected
*/

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
use tokio::sync::Mutex;

use crate::api_manager::{
    models::{send, BridgeState, EventType, PrintInfo, StateWrapper},
    responses::{
        self, bad_request_response, forbidden_response, not_found_response, server_error_response,
    },
};

pub const PATH: &str = "/api/print";
pub const METHODS: &str = "PUT";

pub async fn handler(
    mut req: Request<Body>,
    distributor: Sender<EventType>,
    state: Arc<Mutex<StateWrapper>>,
) -> Response<Body> {
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
    if state.lock().await.state.ne(&BridgeState::CONNECTED) {
        return forbidden_response();
    }

    let path = Path::new("./files/")
        .join(filename)
        .to_string_lossy()
        .into_owned();

    let file = File::open(path.clone());

    if file.is_err() {
        return not_found_response();
    }
    let file = file.unwrap();

    let reader = BufReader::new(file);
    let file_reader = reader.lines();
    let mut gcode: Vec<String> = Vec::with_capacity(file_reader.count() + 1);

    let file = File::open(path);

    if file.is_err() {
        return server_error_response();
    }
    let file = file.unwrap();
    let file_reader = BufReader::new(file).lines();
    let mut size: usize = 0;
    gcode.push("M110 N0".to_string());
    for line in file_reader {
        if line.is_err() {
            return responses::server_error_response();
        }
        let line = line.unwrap();

        let line = line.split_terminator(";").next();
        if line.is_none() {
            continue;
        }
        let line = line.unwrap().trim();
        if line.len() == 0 {
            continue;
        }
        size += line.as_bytes().len();
        gcode.push(line.to_string());
    }
    gcode.shrink_to_fit();

    send(
        &distributor,
        EventType::PrintStart(PrintInfo::new(
            filename.to_string(),
            size,
            gcode,
            Utc::now(),
        )),
    );

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
