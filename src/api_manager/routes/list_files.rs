use std::fs;

use chrono::{DateTime, Utc};
use hyper::{header, Body, Request, Response};
use serde_json::json;

use crate::api_manager::responses::server_error_response;
#[allow(dead_code)]
pub const METHODS: &str = "GET, POST";
pub const PATH: &str = "/api/files";
pub async fn handler(_request: Request<Body>) -> Response<Body> {
    let result = fs::create_dir_all("./files");

    if result.is_err() {
        eprintln!("{}", result.unwrap_err());
        return server_error_response();
    }

    let files = fs::read_dir("./files");
    if result.is_err() {
        return server_error_response();
    }
    let mut json = String::new();

    for file in files.unwrap() {
        match file {
            Ok(file) => {
                if !file.file_name().to_string_lossy().ends_with(".gcode") {
                    continue;
                }
                match file.metadata() {
                    Ok(metadata) => {
                        let date: DateTime<Utc> = metadata.modified().unwrap().into();
                        let size = metadata.len();
                        let row = json!({
                                "name": file.file_name().to_string_lossy(),
                                "uploaded": "test",
                                "uploaded": date,
                                "size": size
                        })
                        .to_string();
                        json = format!("{},{}", json, row);
                    }
                    Err(e) => {
                        eprintln!("[API][LIST_FILES] Error occurred: {}", e);
                        return server_error_response();
                    }
                }
            }
            Err(e) => {
                eprintln!("[API][LIST_FILES] Error occurred: {}", e);
                return server_error_response();
            }
        }
    }

    return Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "X-Requested-With,content-type, Authorization, X-force-upload",
        )
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, PUT")
        .body(Body::from(format!(
            "[{}]",
            json.chars().skip(1).collect::<String>()
        )))
        .expect("Failed to construct valid response");
}
