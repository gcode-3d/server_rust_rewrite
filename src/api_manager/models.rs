use serde::{Deserialize, Serialize};
use sqlx::{sqlite::SqliteRow, FromRow, Row};
#[derive(Serialize, Deserialize, Debug)]
pub struct AuthDetails {
    username: String,
    password: String,
    remember: bool,
}

impl<'r> FromRow<'r, SqliteRow> for AuthDetails {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        Ok(Self {
            username: row.try_get("username")?,
            password: row.try_get("password")?,
            remember: false,
        })
    }
}

impl AuthDetails {
    pub fn is_valid(&self) -> bool {
        if self.username.len() == 0 || self.username.len() > 255 {
            return false;
        } else if self.password.len() == 0 || self.username.len() > 72 {
            return false;
        }
        return true;
    }

    /// Get a reference to the auth details's username.
    pub fn username(&self) -> &str {
        self.username.as_str()
    }

    /// Get a reference to the auth details's password.
    pub fn password(&self) -> &str {
        self.password.as_str()
    }

    /// Get a reference to the auth details's remember.
    pub fn remember(&self) -> &bool {
        &self.remember
    }
}

#[derive(Serialize, Clone)]
pub struct TokenReturnType {
    token: String,
}

