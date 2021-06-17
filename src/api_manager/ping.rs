#[rocket::get("/ping")]
pub fn register() -> &'static str {
    "pong"
}
