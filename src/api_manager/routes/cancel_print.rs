/*
    Send a print end event to the bridge.

    DELETE /api/print

    Permission: print_state.edit
    State: PRINTING
*/

use crossbeam_channel::Sender;
use hyper::{header, Body, Response};

use crate::api_manager::{
    models::{send, BridgeState, EventType, StateWrapper},
    responses::forbidden_response,
};

#[allow(dead_code)]
pub const METHODS: &str = "DELETE";
pub const PATH: &str = "/api/print";

pub fn handler(state_info: StateWrapper, distributor: Sender<EventType>) -> Response<Body> {
    if state_info.state != BridgeState::PRINTING {
        return forbidden_response();
    }

    send(&distributor, EventType::PrintEnd);

    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET,POST,DELETE,PUT")
        .body(Body::empty())
        .expect("Failed to construct valid response");
}
