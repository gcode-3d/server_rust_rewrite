use async_std::task::block_on;
use chrono::{Duration, Utc};
use nickel::{hyper::header, status::StatusCode, JsonBody, Request, Response};
use rand::Rng;
use serde_json::json;
use sqlx::{Connection, Executor, SqliteConnection};

use bcrypt::verify;

use crate::api_manager::models::AuthDetails;

pub fn handler<'a>(request: &mut Request, response: &mut Response) -> String {
    match request.origin.headers.get::<header::ContentType>() {
        Some(value) => {
            if value.to_string() != "application/json" {
                response.set(StatusCode::BadRequest);
                return "".to_string();
            }
        }
        None => {
            response.set(StatusCode::BadRequest);
            return "".to_string();
        }
    }

    match request.json_as::<AuthDetails>() {
        Ok(data) => {
            if data.is_valid() == false {
                response.set(StatusCode::BadRequest);
                return "".to_string();
            }
            let mut connection = block_on(SqliteConnection::connect("storage.db")).unwrap();
            let mut query = sqlx::query_as::<_, AuthDetails>(
                "SELECT username, password FROM users WHERE username = ?",
            );
            query = query.bind(data.username());

            let result = block_on(query.fetch_one(&mut connection));
            match result {
                Ok(row) => {
                    match verify(data.password(), &row.password()) {
                        Ok(result) => {
                            if result == false {
                                response.set(StatusCode::Unauthorized);
                                return "".to_string();
                            } else {
                                return json!({"token": generate_token_for_user(row.username(), !data.remember())}).to_string();
                            }
                        }
                        Err(_) => {
                            todo!();
                        }
                    };
                }
                Err(error) => {
                    if let sqlx::Error::RowNotFound = error {
                        response.set(StatusCode::Unauthorized);
                        return "".to_string();
                    } else {
                        response.set(StatusCode::InternalServerError);
                        return "".to_string();
                    }
                }
            };
        }
        Err(_) => {
            response.set(StatusCode::BadRequest);
            return "".to_string();
        }
    }
}

fn generate_token_for_user(username: &str, does_expire: bool) -> String {
    let token = rand::thread_rng()
        .sample_iter(&rand::distributions::Alphanumeric)
        .take(60)
        .map(char::from)
        .collect();

    let mut connection = block_on(SqliteConnection::connect("storage.db")).unwrap();
    let mut query = sqlx::query("INSERT INTO tokens (username, token, expire) values (?, ?, ?)");
    query = query.bind(username);
    query = query.bind(&token);

    query = if does_expire {
        let time = Utc::now() + Duration::hours(6);
        query.bind(time.to_rfc3339())
    } else {
        query.bind(Option::<String>::None)
    };
    block_on(connection.execute(query)).unwrap();
    return token;
}
