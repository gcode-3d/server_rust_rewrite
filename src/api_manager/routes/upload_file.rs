/*
    Upload a .gcode file to the files directory.

    ! Cannot upload a file that is currently printing.

    POST /api/files
    multipart/form-data

    Permission: files.edit
    State: -
*/

use std::{
    borrow::Borrow,
    fs::{self, File},
    io::Write,
    path::Path,
    sync::Arc,
    usize,
};

use futures::StreamExt;
use hyper::{header, Body, Request, Response, StatusCode};
use regex::Regex;
use tokio::sync::Mutex;

use crate::api_manager::{
    models::{StateDescription, StateWrapper},
    responses::{
        self, bad_request_response, forbidden_response, server_error_response, too_large_response,
    },
};
use lazy_static::lazy_static;

pub const METHODS: &str = "GET, POST";
pub const PATH: &str = "/api/files";

lazy_static! {
    static ref NAME_REGEX: Regex =
        Regex::new(r#"Content-Disposition: form-data; name="file"; filename="([^\\/.]*\.gcode)""#,)
            .unwrap();
}

pub async fn handler(
    req: &mut Request<Body>,
    state_info: Arc<Mutex<StateWrapper>>,
) -> Response<Body> {
    if !req.headers().contains_key("content-type") {
        return bad_request_response();
    }
    if !req
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .starts_with("multipart/form-data; boundary=")
    {
        return bad_request_response();
    }

    if !req.headers().contains_key("content-length") {
        return bad_request_response();
    }
    let promised_size: usize = req
        .headers()
        .get("content-length")
        .unwrap()
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    if promised_size > 200000000 {
        return responses::too_large_response();
    }

    let folder_create_result = fs::create_dir_all("./files");

    if folder_create_result.is_err() {
        return responses::server_error_response();
    }

    let mut bytes: usize = 0;
    let boundary = req
        .headers()
        .get("content-type")
        .unwrap()
        .to_str()
        .unwrap()
        .replacen("multipart/form-data; boundary=", "", 1);

    let mut file: Option<File> = None;
    let mut is_capturing = false;

    while let Some(chunk) = req.body_mut().next().await {
        let data = chunk.unwrap();
        bytes += data.len();
        match String::from_utf8(data.to_vec()) {
            Ok(mut data) => {
                let regex = Regex::new(r"\r\n?|\n").unwrap();
                data = regex.replace_all(&data, "\n").to_string();

                if data.starts_with(&format!("--{}", boundary)) && data.contains("\n\n") {
                    let header = data.split("\n\n").next().unwrap();
                    let captures = NAME_REGEX.captures(header);
                    if captures.is_some() {
                        let capture = captures.unwrap().get(1);
                        if capture.is_some() {
                            let name = capture.unwrap().as_str();
                            let state = state_info.lock().await;
                            match &state.description {
                                StateDescription::Print {
                                    filename,
                                    progress: _,
                                    start: _,
                                    end: _,
                                } => {
                                    if filename == name {
                                        return forbidden_response();
                                    }
                                }
                                _ => (),
                            }

                            match File::create(Path::new("./files/").join(name)) {
                                Ok(created_file) => {
                                    file = Some(created_file);
                                    is_capturing = true;
                                    data = data.replacen(header, "", 1).trim_start().to_string();
                                }
                                Err(e) => {
                                    eprintln!("[API][STOREFILE] Error: {}", e);
                                    return server_error_response();
                                }
                            }
                        } else {
                            return bad_request_response();
                        }
                    } else {
                        return bad_request_response();
                    }
                }
                if file.borrow().is_none() {
                    return bad_request_response();
                }

                {
                    let pattern = format!("\n--{}--", boundary);
                    if data.split(&pattern).count() > 1 && is_capturing {
                        data = data.split(&pattern).next().unwrap().to_string();
                        is_capturing = false;
                    }
                }

                if data.len() > 0 {
                    let bytes = data.as_bytes();
                    match file.as_ref().unwrap().write(bytes) {
                        Ok(bytes_written) => {
                            if bytes_written != bytes.len() {
                                return server_error_response();
                            }
                        }
                        Err(e) => {
                            eprintln!("[API][STOREFILE] Error: {}", e);
                        }
                    }
                }
            }
            Err(_) => {
                return bad_request_response();
            }
        }
    }

    if bytes.gt(&promised_size) {
        return too_large_response();
    } else if bytes.lt(&promised_size) {
        return bad_request_response();
    }

    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .status(StatusCode::CREATED)
        .body(Body::from("Created"))
        .expect("Failed to construct valid response");
}
