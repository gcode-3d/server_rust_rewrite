mod models;
mod routes;

use nickel::{middleware, HttpRouter, Nickel};

pub struct ApiManager {
    pub server: Nickel,
}

impl ApiManager {
    pub async fn new() -> Self {
        let mut manager = Self {
            server: Nickel::new(),
        };
        manager.setup_routes();
        return manager;
    }

    fn setup_routes(&mut self) {
        self.server
            .get("/api/ping", middleware!(routes::ping::handler()));

        self.server.post(
            "/api/login",
            middleware!(|request, mut response| routes::login::handler(request, &mut response)),
        );
    }
}
