pub struct User {
    pub username: String,
    pub password_hash: String,
}

impl User {
    pub fn new(username: String, password_hash: String) -> Self {
        Self {
            username,
            password_hash: password_hash,
        }
    }
}
