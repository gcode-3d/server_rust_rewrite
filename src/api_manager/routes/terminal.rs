/*
    Sends a message to the printer.

    POST /api/terminal

    Body: (json)
        message: String


    Permission: terminal
    State: Connected
*/

use crate::{
    api_manager::{
        models::{BridgeEvents, EventInfo, EventType},
        responses::{bad_request_response, forbidden_response},
    },
    bridge::BridgeState,
};
use crossbeam_channel::Sender;
use hyper::{body, header, Body, Request, Response};
use serde_json::{json, Value};
use uuid::Uuid;

pub const METHODS: &str = "POST";
pub const PATH: &str = "/api/terminal";

pub async fn handler(
    mut request: Request<Body>,
    sender: Sender<EventInfo>,
    state: BridgeState,
) -> Response<Body> {
    if state != BridgeState::CONNECTED {
        return forbidden_response();
    }
    let result = body::to_bytes(request.body_mut()).await.unwrap();
    let body = match String::from_utf8(result.to_vec()) {
        Ok(body) => Some(body),
        Err(e) => {
            eprintln!("[API][TERMINAL_SEND] Invalid body received: {}", e);
            None
        }
    };

    if body.is_none() {
        return bad_request_response();
    }
    let json = serde_json::from_str::<Value>(&body.unwrap());
    if json.is_err() {
        return bad_request_response();
    }
    let json = json.unwrap();
    let message = json.get("message");
    if message.is_none() {
        return bad_request_response();
    }
    let message = message.unwrap().as_str();
    if message.is_none() {
        return bad_request_response();
    }
    let message = message.unwrap();
    let id = Uuid::new_v4();
    sender
        .send(EventInfo {
            event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                message: message.to_string(),
                id: id.clone(),
            }),
        })
        .expect("Cannot send message");

    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .body(Body::from(
            json!({"id": id.to_hyphenated().to_string()}).to_string(),
        ))
        .expect("Failed to construct valid response");
}
