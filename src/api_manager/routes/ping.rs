/*
    Returns a simple response to test if online.

    GET /api/ping

    Permission: -
    State: -
*/
use hyper::{header, Body, Request, Response};

pub const METHODS: &str = "GET";
pub const PATH: &str = "/api/ping";

pub fn handler(_request: Request<Body>) -> Response<Body> {
    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET")
        .body(Body::from("Pong!"))
        .expect("Failed to construct valid response");
}
