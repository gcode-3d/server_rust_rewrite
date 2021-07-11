use hyper::{header, Body, Response, StatusCode};

pub fn not_found_response() -> Response<Body> {
    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .status(StatusCode::NOT_FOUND)
        .body(Body::from("Not Found"))
        .expect("Failed to construct a valid response");
}

pub fn bad_request_response() -> Response<Body> {
    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .status(StatusCode::BAD_REQUEST)
        .body(Body::from("Bad Request"))
        .expect("Failed to construct a valid response");
}

pub fn unauthorized_response() -> Response<Body> {
    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .status(StatusCode::UNAUTHORIZED)
        .body(Body::from("Bad Request"))
        .expect("Failed to construct a valid response");
}

pub fn server_error_response() -> Response<Body> {
    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .status(StatusCode::INTERNAL_SERVER_ERROR)
        .body(Body::from("Internal Server Error"))
        .expect("Failed to construct a valid response");
}
