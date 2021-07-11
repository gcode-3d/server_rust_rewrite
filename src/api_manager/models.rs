use serde::{Deserialize, Serialize};

use sqlx::{sqlite::SqliteRow, FromRow, Row};
#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Clone, Debug)]
pub struct SettingRow {
    pub id: String,
    pub raw_value: String,
    pub row_type: u8,
    pub bool: Option<bool>,
    pub number: Option<u64>,
    pub float: Option<f64>,
}

impl<'r> FromRow<'r, SqliteRow> for SettingRow {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        let id = row.try_get("id")?;
        let value: String = row.try_get("value")?;
        let row_type = row.try_get("type")?;
        let mut bool = None;
        let mut number = None;
        let mut float = None;

        if row_type == 1 {
            if value == "1" {
                bool = Some(true);
            } else {
                bool = Some(false);
            }
        } else if row_type == 2 {
            if value.len() == 0 {
                number = Some(0);
            } else {
                number = Some(value.parse::<u64>().unwrap());
            }
        } else if row_type == 3 {
            if value.len() == 0 {
                float = Some(0.0);
            } else {
                float = Some(value.parse::<f64>().unwrap());
            }
        }
        return Ok(Self {
            id,
            raw_value: value,
            row_type,
            bool,
            number,
            float,
        });
    }
}

#[derive(Clone, Debug)]
pub struct AuthPermissions {
    username: String,
    raw_permissions: u8,
    admin: bool,
    edit_connection: bool,
    file_access: bool,
    file_edit: bool,
    print_state_edit: bool,
    settings_edit: bool,
    users_edit: bool,
    terminal_read: bool,
    terminal_send: bool,
    webcam: bool,
    update: bool,
}

impl AuthPermissions {
    /// Get a reference to the auth permissions's username.
    pub fn username(&self) -> &str {
        self.username.as_str()
    }

    /// Get a reference to the auth permissions's admin.
    pub fn admin(&self) -> &bool {
        &self.admin
    }

    /// Get a reference to the auth permissions's edit connection.
    pub fn edit_connection(&self) -> &bool {
        &self.edit_connection
    }

    /// Get a reference to the auth permissions's file access.
    pub fn file_access(&self) -> &bool {
        &self.file_access
    }

    /// Get a reference to the auth permissions's file edit.
    pub fn file_edit(&self) -> &bool {
        &self.file_edit
    }

    /// Get a reference to the auth permissions's print state edit.
    pub fn print_state_edit(&self) -> &bool {
        &self.print_state_edit
    }

    /// Get a reference to the auth permissions's settings edit.
    pub fn settings_edit(&self) -> &bool {
        &self.settings_edit
    }

    /// Get a reference to the auth permissions's users edit.
    pub fn users_edit(&self) -> &bool {
        &self.users_edit
    }

    /// Get a reference to the auth permissions's terminal read.
    pub fn terminal_read(&self) -> &bool {
        &self.terminal_read
    }

    /// Get a reference to the auth permissions's terminal send.
    pub fn terminal_send(&self) -> &bool {
        &self.terminal_send
    }

    /// Get a reference to the auth permissions's webcam.
    pub fn webcam(&self) -> &bool {
        &self.webcam
    }

    /// Get a reference to the auth permissions's update.
    pub fn update(&self) -> &bool {
        &self.update
    }
}

impl<'r> FromRow<'r, SqliteRow> for AuthPermissions {
    fn from_row(row: &SqliteRow) -> Result<Self, sqlx::Error> {
        let raw_permissions = row.try_get("permissions")?;
        let binary = format!("000000000000{:b}", &raw_permissions);
        let mut reversed = vec![];
        for b in (0..binary.as_bytes().len()).rev() {
            reversed.push(binary.as_bytes()[b]);
        }

        Ok(Self {
            username: row.try_get("username")?,
            raw_permissions,
            admin: reversed[0] == 49,
            edit_connection: reversed[0] == 49 || reversed[1] == 49,
            file_access: reversed[0] == 49 || reversed[2] == 49,
            file_edit: reversed[0] == 49 || reversed[3] == 49,
            print_state_edit: reversed[0] == 49 || reversed[4] == 49,
            settings_edit: reversed[0] == 49 || reversed[5] == 49,
            users_edit: reversed[0] == 49 || reversed[6] == 49,
            terminal_read: reversed[0] == 49 || reversed[7] == 49,
            terminal_send: reversed[0] == 49 || reversed[8] == 49,
            webcam: reversed[0] == 49 || reversed[9] == 49,
            update: reversed[0] == 49 || reversed[10] == 49,
        })
    }
}

#[derive(Serialize, Clone)]
pub struct TokenReturnType {
    token: String,
}

#[derive(Clone, Debug)]
pub struct EventInfo {
    pub event_type: EventType,
    pub message_data: String,
}

#[derive(Clone, Debug)]
pub enum EventType {
    Bridge(BridgeEvents),
    Websocket(WebsocketEvents),
}
#[derive(Clone, Debug)]
pub enum WebsocketEvents {
    TerminalSend { message: String },
    StateUpdate { state: State },
}

#[derive(Clone, Debug)]
pub enum BridgeEvents {
    ConnectionCreate { address: String, port: u32 },
    ConnectionCreateError { error: u8 },
    Ping,
}

#[derive(Clone, Debug)]
pub enum State {
    Disconnected,
    Connecting,
    Connected,
    Errored { description: String },
}
