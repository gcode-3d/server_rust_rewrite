use chrono::{DateTime, Utc};
use crossbeam_channel::Sender;
use serde::{Deserialize, Serialize};

use sqlx::{sqlite::SqliteRow, FromRow, Row};
use uuid::Uuid;

use crate::parser::TempInfo;

pub fn send(sender: &Sender<EventType>, data: EventType) {
    let result = sender.send(data);
    if result.is_err() {
        eprintln!("[ERROR] {}", result.unwrap_err());
    }
}

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
            if value == "1" || value == "true" {
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

#[derive(Debug)]
pub struct EventInfo {
    pub event_type: EventType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum BridgeState {
    DISCONNECTED = 0,
    CONNECTED = 1,
    CONNECTING = 2,
    ERRORED = -1,
    PREPARING = 5,
    PRINTING = 6,
    FINISHING = 7,
}

#[derive(Debug)]
pub enum EventType {
    StateUpdate(StateWrapper),
    CreateBridge {
        address: String,
        port: u32,
    },
    CreateBridgeError {
        error: String,
    },
    KillBridge,
    PrintEnd,
    PrintStart(PrintInfo),
    TempUpdate {
        tools: Vec<TempInfo>,
        bed: Option<TempInfo>,
        chamber: Option<TempInfo>,
    },
    IncomingTerminalMessage(String),
    OutGoingTerminalMessage(Message),
}

impl std::fmt::Display for EventType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EventType::StateUpdate(StateWrapper {
                state,
                description: _,
            }) => {
                write!(f, "Stateupdate event - {:?}", state)
            }

            EventType::CreateBridge { address, port } => {
                write!(f, "Create bridge event: {}:{}", address, port)
            }
            EventType::CreateBridgeError { error } => {
                write!(f, "Create bridge error event: {}", error)
            }
            EventType::KillBridge => {
                write!(f, "Kill bridge event")
            }
            EventType::PrintEnd => {
                write!(f, "End print event")
            }
            EventType::PrintStart(info) => {
                write!(f, "Start print event {}", info.filename)
            }
            EventType::TempUpdate {
                tools: _,
                bed: _,
                chamber: _,
            } => {
                write!(f, "Temp update event ")
            }
            EventType::IncomingTerminalMessage(message) => {
                write!(f, "Incoming terminal message event | {}", message)
            }
            EventType::OutGoingTerminalMessage(message) => {
                write!(f, "Outgoing terminal message event | {:?}", message)
            }
        }
    }
}

#[derive(Debug)]
pub struct PrintInfo {
    pub filename: String,
    pub filesize: usize,
    data_sent: u64,
    // pub file_reader: Option<Lines<BufReader<File>>>,
    pub gcode: Vec<String>,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    line_number: usize,
    resend_amount: usize,
}

impl PrintInfo {
    pub fn new(
        filename: String,
        filesize: usize,
        // file_reader: Option<Lines<BufReader<File>>>,
        gcode: Vec<String>,
        start: DateTime<Utc>,
    ) -> Self {
        Self {
            filename,
            filesize,
            data_sent: 0,
            gcode,
            // file_reader,
            start,
            end: None,
            line_number: 0,
            resend_amount: 0,
        }
    }
    pub fn report_resend(&mut self) {
        self.resend_amount += 1;
    }
    pub fn get_resend_ratio(&self) -> f32 {
        return (self.resend_amount as f32 / self.gcode.len() as f32) * 100.0;
    }
    pub fn get_resend_amount(&self) -> &usize {
        return &self.resend_amount;
    }
    pub fn get_line_amount(&self) -> usize {
        return self.gcode.len();
    }
    pub fn get_line_by_index(&self, index: usize) -> Option<Line> {
        let content = self.get_line_content_by_index(index);
        if content.is_none() {
            return None;
        }
        return Some(Line::new(content.unwrap().clone(), index));
    }
    fn get_line_content_by_index(&self, index: usize) -> Option<&String> {
        return self.gcode.get(index);
    }

    pub fn set_line_number(&mut self, line_number: usize) {
        if line_number > self.gcode.len() + 1 {
            panic!("Cannot set line number out of bounds");
        }
        self.line_number = line_number;
    }
    pub fn progress(&self) -> f64 {
        if self.filesize == 0 {
            return 0.0;
        }
        return (self.data_sent as f64 / self.filesize as f64) * 100.0;
    }

    pub fn add_bytes_sent(&mut self, bytes: u64) {
        if self.filesize == 0 {
            return;
        }
        self.data_sent = self.data_sent + bytes;
    }

    pub fn line_number(&mut self) -> usize {
        return self.line_number;
    }
}

#[derive(Debug, Clone)]
pub struct Line {
    content: String,
    line_number: usize,
}

impl Line {
    pub fn new(content: String, line_number: usize) -> Self {
        Self {
            content,
            line_number,
        }
    }

    /// Get a reference to the line's line.
    pub fn content(&self) -> &str {
        self.content.as_str()
    }

    /// Get a reference to the line's line number.
    pub fn line_number(&self) -> &usize {
        &self.line_number
    }
}

#[derive(Debug, Clone)]
pub struct StateWrapper {
    pub state: BridgeState,
    pub description: StateDescription,
}

#[derive(Debug, Clone)]
pub enum StateDescription {
    None,
    Capability {
        capabilities: Vec<String>,
    },
    Error {
        message: String,
    },
    Print {
        filename: String,
        progress: f64,
        start: DateTime<Utc>,
        end: Option<DateTime<Utc>>,
    },
}

#[derive(Clone, Debug)]
pub struct Message {
    pub content: String,
    pub id: Uuid,
}

impl Message {
    pub fn new(content: String, id: Uuid) -> Self {
        return Self { content, id };
    }
}

#[derive(Clone, Debug)]
pub enum BridgeAction {
    Continue(Option<usize>),
    Error,
    Resend(usize),
}

impl std::fmt::Display for BridgeAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BridgeAction::Continue(line) => {
                if line.is_some() {
                    write!(f, "Continue with N{}", line.unwrap_or(0))
                } else {
                    write!(f, "Continue")
                }
            }
            BridgeAction::Error => write!(f, "Error"),
            BridgeAction::Resend(line) => {
                write!(f, "Resend N{}", line)
            }
        }
    }
}
