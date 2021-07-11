use hyper::{header, Body, Request, Response};

pub fn handler(_request: Request<Body>) -> Response<Body> {
    // println!("{:?}", request.headers());
    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, "GET")
        .body(Body::from("Pong!"))
        .expect("Failed to construct valid response");
}
