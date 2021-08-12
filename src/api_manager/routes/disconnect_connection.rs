use crossbeam_channel::Sender;
use hyper::{header, Body, Response};
use serde_json::json;

use crate::{
    api_manager::models::{BridgeEvents, EventInfo, EventType, StateDescription},
    bridge::BridgeState,
};
pub const METHODS: &str = "PUT, DELETE, POST";
pub const PATH: &str = "/api/connection";

pub async fn handler(state: BridgeState, distributor: Sender<EventInfo>) -> Response<Body> {
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

    distributor
        .send(EventInfo {
            event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                state: BridgeState::DISCONNECTED,
                description: StateDescription::None,
            }),
        })
        .expect("Cannot send message");

    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .body(Body::empty())
        .expect("Failed to construct valid response");
}
