use rocket::request::FromRequest;
use rocket::{
    http::Status,
    request::{Outcome, Request},
};
use uuid::Uuid;

#[rocket::post("/login")]
pub fn register() -> &'static str {
    println!("test?");
    return "test";
}

struct AuthenticatedUser {
    user_id: Uuid,
}

#[derive(Debug)]
enum LoginError {
    InvalidData,
    UsernameDoesNotExist,
    WrongPassword,
}

impl<'r> FromRequest<'r> for AuthenticatedUser {
    type Error = LoginError;
    fn from_request(request: &'r Request) -> Outcome<AuthenticatedUser, LoginError> {
        let jar = request.cookies();
        for cookie in jar.iter() {
            println!("{} => {}", cookie.name(), cookie.value());
        }
        return Outcome::Failure((Status::Unauthorized, LoginError::WrongPassword));
    }
}
