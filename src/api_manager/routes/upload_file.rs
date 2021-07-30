use std::{
    borrow::Borrow,
    fs::{self, File},
    io::Write,
    path::Path,
    usize,
};

use futures::StreamExt;
use hyper::{header, Body, Request, Response, StatusCode};
use regex::Regex;
use sqlx::{Connection, SqliteConnection};

use crate::api_manager::{
    models::AuthPermissions,
    responses::{
        self, bad_request_response, server_error_response, too_large_response,
        unauthorized_response,
    },
};

pub const METHODS: &str = "GET, POST";
pub const PATH: &str = "/api/files";

pub async fn handler(req: &mut Request<Body>) -> Response<Body> {
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

    if !req.headers().contains_key("authorization") {
        return unauthorized_response();
    }
    let token = req
        .headers()
        .get("authorization")
        .unwrap()
        .to_str()
        .expect("Invalid token");

    if token.len() != 60 || !token.chars().all(char::is_alphanumeric) {
        return unauthorized_response();
    }

    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    let mut query = sqlx::query_as::<_, AuthPermissions>(
									"select a.username as username, a.permissions as permissions from users a inner join tokens b on a.username = b.username where (b.expire < DATE('now') OR b.expire is null) AND b.token = ?",
							);

    query = query.bind(token);

    match query.fetch_one(&mut connection).await {
        Ok(user) => {
            if !user.file_edit() || !user.file_access() {
                return responses::forbidden_response();
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
                        let name_regex = Regex::new(r#"Content-Disposition: form-data; name="file"; filename="([^\\]*\.gcode)""#).unwrap();
                        if data.starts_with(&format!("--{}", boundary)) && data.contains("\n\n") {
                            let header = data.split("\n\n").next().unwrap();
                            let captures = name_regex.captures(header);
                            if captures.is_some() {
                                let capture = captures.unwrap().get(1);
                                if capture.is_some() {
                                    match File::create(
                                        Path::new("./files/").join(capture.unwrap().as_str()),
                                    ) {
                                        Ok(created_file) => {
                                            file = Some(created_file);
                                            is_capturing = true;
                                            data = data
                                                .replacen(header, "", 1)
                                                .trim_start()
                                                .to_string();
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
        Err(_) => {
            return unauthorized_response();
        }
    };
}
