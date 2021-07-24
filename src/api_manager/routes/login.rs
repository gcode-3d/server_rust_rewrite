use chrono::{Duration, Utc};
use hyper::{
    body::{self},
    header, Body, Request, Response, StatusCode,
};
use rand::Rng;
use serde_json::json;
use sqlx::{Connection, Executor, SqliteConnection};

use bcrypt::verify;

use crate::api_manager::{
    models::AuthDetails,
    responses::{bad_request_response, server_error_response, unauthorized_response},
};

pub const PATH: &str = "/api/login";
pub const METHODS: &str = "POST";
pub async fn handler(mut request: Request<Body>) -> Response<Body> {
    if !request.headers().contains_key(header::CONTENT_TYPE) {
        return bad_request_response();
    }
    match request.headers().get(header::CONTENT_TYPE) {
        Some(value) => {
            if value.ne("application/json") {
                return bad_request_response();
            }
        }
        None => return bad_request_response(),
    };
    let auth_info = get_auth_from_body(request.body_mut()).await;

    if let Some(details) = auth_info {
        if !details.is_valid() {
            return bad_request_response();
        }
        let mut connection = SqliteConnection::connect("storage.db").await.unwrap();
        let mut query = sqlx::query_as::<_, AuthDetails>(
            "SELECT username, password FROM users WHERE username = ?",
        );
        query = query.bind(details.username());
        let result = query.fetch_one(&mut connection).await;
        match result {
            Ok(row) => match verify(details.password(), row.password()) {
                Ok(result) => {
                    if !result {
                        return unauthorized_response();
                    }
                    let data = json!(
                            {
                                "token": generate_token_for_user(row.username(), !details.remember()).await
                        }
                    ).to_string();
                    return Response::builder()
                        .status(StatusCode::CREATED)
                        .body(Body::from(data))
                        .expect("Failed to construct response");
                }
                Err(e) => {
                    eprintln!("hash verify error: {}", e);
                    return server_error_response();
                }
            },
            Err(e) => match e {
                sqlx::Error::RowNotFound => {
                    return unauthorized_response();
                }
                _ => {
                    eprintln!("sql error: {}", e);
                    return server_error_response();
                }
            },
        }
    } else {
        return bad_request_response();
    }
}

async fn get_auth_from_body(body: &mut Body) -> Option<AuthDetails> {
    let result = body::to_bytes(body).await;
    match result {
        Ok(bytes) => match String::from_utf8(bytes.to_vec()) {
            Ok(value) => match serde_json::from_str::<AuthDetails>(&value) {
                Ok(auth) => return Some(auth),
                Err(_) => return None,
            },
            Err(_) => return None,
        },
        Err(_) => return None,
    }
}

async fn generate_token_for_user(username: &str, does_expire: bool) -> String {
    let token = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(60)
        .map(char::from)
        .collect();

    let mut connection = SqliteConnection::connect("storage.db").await.unwrap();
    let mut query = sqlx::query("INSERT INTO tokens (username, token, expire) values (?, ?, ?)");
    query = query.bind(username);
    query = query.bind(&token);

    query = if does_expire {
        let time = Utc::now() + Duration::hours(24);
        query.bind(time.to_rfc3339())
    } else {
        query.bind(Option::<String>::None)
    };
    connection.execute(query).await.unwrap();
    return token;
}
