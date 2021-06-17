mod login;
mod models;
mod ping;

use rocket::get;
use rocket::launch;
use rocket::routes;

#[get("/")]
fn index() -> &'static str {
    "Hello, world!"
}

#[launch]
pub fn rocket() -> _ {
    rocket::build()
        // .register("/", rocket::catchers![not_found])
        .mount("/api", routes![index, ping::register, login::register])
}

/*
#[rocket::catch(404)]
fn not_found() -> &'static str {
    println!("Request received 404");
    "404 Not Found"
}
*/
