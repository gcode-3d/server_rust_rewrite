/*
    Returns the sentry dsn for localstorage.

    GET /api/dsn

    Permission: -
    State: -
*/

use hyper::{header, Body, Response};
use serde::Deserialize;
use serde_json::json;
use sqlx::{Connection, SqliteConnection};

use crate::api_manager::{models::SettingRow, responses::bad_request_response};

pub const PATH: &str = "/api/sentry/dsn";
pub const METHODS: &str = "GET";

pub async fn handler() -> Response<Body> {
    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    let query = sqlx::query_as::<_, SettingRow>("SELECT * FROM settings where id = 'S_sentryDsn'");

    let result = query.fetch_one(&mut connection).await;
    if result.is_err() {
        return bad_request_response();
    }
    let row = result.unwrap();

    return Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(
            header::ACCESS_CONTROL_ALLOW_HEADERS,
            "Authorization, Content-Type",
        )
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .body(Body::from(json!({"dsn": row.raw_value}).to_string()))
        .expect("Failed to construct valid response");
}

#[derive(Deserialize, Debug)]
#[allow(non_snake_case)]
struct JsonSettingRow {
    pub settingName: String,
    pub settingValue: String,
}
