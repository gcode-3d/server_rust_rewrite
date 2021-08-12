use hyper::{header, Body, Request, Response};
use serde_json::Value;
use sqlx::{Connection, SqliteConnection};

use crate::api_manager::{
    models::{AuthPermissions, SettingRow},
    responses::{server_error_response, unauthorized_response},
};

pub const PATH: &str = "/api/settings";
pub const METHODS: &str = "GET, POST";

pub async fn handler(req: Request<Body>) -> Response<Body> {
    if !req.headers().contains_key("authorization") {
        return unauthorized_response();
    }

    let token = req
        .headers()
        .get("authorization")
        .unwrap()
        .to_str()
        .expect("Not a valid value");

    if token.len() != 60 || !token.chars().all(char::is_alphanumeric) {
        return unauthorized_response();
    }
    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    let mut query = sqlx::query_as::<_, AuthPermissions>(
                "select a.username as username, a.permissions as permissions from users a inner join tokens b on a.username = b.username where (b.expire < DATE('now') OR b.expire is null) AND b.token = ?",
            );

    query = query.bind(token);

    let result = query.fetch_one(&mut connection).await;

    if result.is_err() {
        return unauthorized_response();
    }

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
