/*
    Send a event to the bridge to start disconnecting connection.

    DELETE /api/connection

    Permission: connection.edit
    State: Connected | Printing
*/

use crossbeam_channel::Sender;
use hyper::{header, Body, Response};
use serde_json::json;

use crate::api_manager::models::{send, BridgeState, EventType, StateDescription, StateWrapper};
pub const METHODS: &str = "PUT, DELETE, POST";
pub const PATH: &str = "/api/connection";

pub async fn handler(state: BridgeState, distributor: Sender<EventType>) -> Response<Body> {
    if state.eq(&BridgeState::DISCONNECTED) || state.eq(&BridgeState::ERRORED) {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/plain")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
            .status(403)
            .body(Body::from(
                json!({"error": true, "message": "Not connected"}).to_string(),
            ))
            .expect("Failed to construct valid response");
    }

    send(
        &distributor,
        EventType::StateUpdate(StateWrapper {
            state: BridgeState::DISCONNECTED,
            description: StateDescription::None,
        }),
    );

    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .body(Body::empty())
        .expect("Failed to construct valid response");
}
