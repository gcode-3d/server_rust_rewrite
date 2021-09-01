/*
    Update the specified setting with the provided value.

    POST /api/settings

    Permission: settings.edit
    State: -
*/

use hyper::{body, header, Body, Request, Response};
use serde::Deserialize;
use sqlx::{Connection, SqliteConnection};

use crate::api_manager::responses::bad_request_response;

pub const PATH: &str = "/api/settings";
pub const METHODS: &str = "GET, POST";

pub async fn handler(mut req: Request<Body>) -> Response<Body> {
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

    let json_result = serde_json::from_str::<JsonSettingRow>(&body.unwrap());
    if json_result.is_err() {
        return bad_request_response();
    }

    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    let mut query = sqlx::query("update settings set value = ? where id = ?");
    let json = json_result.unwrap();

    query = query.bind(json.settingValue);
    query = query.bind(json.settingName);

    let result = query.execute(&mut connection).await;
    if result.is_err() {
        println!("{}", result.unwrap_err());
        return bad_request_response();
    }
    return Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Authorization, Content-Type",
        )
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .body(Body::empty())
        .expect("Failed to construct valid response");
}

#[derive(Deserialize, Debug)]
#[allow(non_snake_case)]
struct JsonSettingRow {
    pub settingName: String,
    pub settingValue: String,
}
