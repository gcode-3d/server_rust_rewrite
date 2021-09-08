/*
    Rename a gcode file.

    ! Cannot rename a file that is currently printing.

    POST /api/files/rename

    Permission: files.edit
    State: -
*/

use std::{path::Path, sync::Arc};

use hyper::{body, header, Body, Request, Response};
use regex::Regex;
use tokio::sync::Mutex;

use crate::api_manager::{
    models::{StateDescription, StateWrapper},
    responses::{bad_request_response, forbidden_response, server_error_response},
};
use lazy_static::lazy_static;
use serde::Deserialize;

lazy_static! {
    static ref NAME_REGEX: Regex = Regex::new(r#"^[^\\./]*\.gcode$"#,).unwrap();
}

pub const METHODS: &str = "POST";
pub const PATH: &str = "/api/files/rename";

pub async fn handler(
    mut request: Request<Body>,
    state: Arc<Mutex<StateWrapper>>,
) -> Response<Body> {
    let result = body::to_bytes(request.body_mut()).await.unwrap();
    let body = match String::from_utf8(result.to_vec()) {
        Ok(body) => Some(body),
        Err(e) => {
            eprintln!("[API][Rename_file] Invalid body received: {}", e);
            None
        }
    };

    if body.is_none() {
        return bad_request_response();
    }
    let json = serde_json::from_str::<RenameFileBody>(&body.unwrap());
    if json.is_err() {
        return bad_request_response();
    }
    let json = json.unwrap();

    let state = state.lock().await;

    if let StateDescription::Print {
        filename,
        progress: _,
        start: _,
        end: _,
    } = state.description.clone()
    {
        if filename == json.new_name || filename == json.old_name {
            return forbidden_response();
        }
    }
    if !NAME_REGEX.is_match(&json.new_name) || !NAME_REGEX.is_match(&json.old_name) {
        return bad_request_response();
    }

    let result = std::fs::rename(
        Path::new("./files").join(json.old_name),
        Path::new("./files").join(json.new_name),
    );
    if result.is_err() {
        return server_error_response();
    }
    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .body(Body::empty())
        .expect("Failed to construct valid response");
}

#[derive(Debug, Deserialize)]
struct RenameFileBody {
    new_name: String,
    old_name: String,
}
