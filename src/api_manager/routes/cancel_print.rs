use crossbeam_channel::Sender;
use hyper::{header, Body, Response};

use crate::{
    api_manager::{
        models::{BridgeEvents, EventInfo, EventType, StateWrapper},
        responses::{forbidden_response, server_error_response},
    },
    bridge::BridgeState,
};

pub const METHODS: &str = "DELETE";
pub const PATH: &str = "/api/print/";

pub fn handler(state_info: StateWrapper, distributor: Sender<EventInfo>) -> Response<Body> {
    if state_info.state != BridgeState::PRINTING {
        return forbidden_response();
    }

    let result = distributor.send(EventInfo {
        event_type: EventType::Bridge(BridgeEvents::PrintEnd),
    });
    if result.is_err() {
        let error = result.unwrap_err();
        let msg = format!("{}", error);
        eprintln!("[API][ERROR][CANCELPRINT] {}", error);
        sentry::capture_message(&msg, sentry::Level::Error);
        return server_error_response();
    }

    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET,POST,DELETE,PUT")
        .body(Body::empty())
        .expect("Failed to construct valid response");
}
