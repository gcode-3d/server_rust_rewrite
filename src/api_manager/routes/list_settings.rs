use hyper::{header, Body, Response};
use serde_json::Value;
use sqlx::{Connection, SqliteConnection};

use crate::api_manager::{models::SettingRow, responses::server_error_response};

pub const PATH: &str = "/api/settings";
pub const METHODS: &str = "GET, POST";

pub async fn handler() -> Response<Body> {
    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    let query = sqlx::query_as::<_, SettingRow>("select * from settings");
    let result = query.fetch_all(&mut connection).await;
    if result.is_err() {
        return server_error_response();
    }

    // let mut json = String::new();
    let mut map = serde_json::Map::new();
    for row in result.unwrap() {
        if row.row_type == 0 {
            map.insert(row.id, Value::String(row.raw_value));
        } else if row.row_type == 1 {
            map.insert(row.id, Value::Bool(row.bool.unwrap()));
        } else if row.row_type == 2 {
            map.insert(row.id, Value::from(row.number.unwrap()));
        } else if row.row_type == 3 {
            map.insert(row.id, Value::from(row.float.unwrap()));
        }
    }
    let json = Value::Object(map);

    return Response::builder()
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_HEADERS, "Authorization")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET, POST")
        .body(Body::from(json.to_string()))
        .expect("Failed to construct valid response");
}
