use std::{collections::HashMap, sync::Arc, time::Duration};

use crate::api_manager::models::{send, EventType, StateWrapper, BridgeState};
use api_manager::{
    models::{SettingRow, StateDescription},
    ApiManager,
};

use bridge::Bridge;
use chrono::{DateTime, Utc};
use crossbeam_channel::{unbounded, Receiver, Sender};
use futures::SinkExt;
use hyper::upgrade::Upgraded;
use hyper_tungstenite::{tungstenite::Message, WebSocketStream};
use serde_json::json;
use sqlx::{Connection, Executor, SqliteConnection};
use tokio::{
    fs::OpenOptions,
    spawn,
    sync::Mutex,
    task::{yield_now, JoinHandle},
    time::{sleep, Instant},
};
use uuid::Uuid;
mod api_manager;
mod bridge;
mod client_update_check;
mod parser;

#[tokio::main(worker_threads = 2)]
async fn main() {
    let _guard = sentry::init((
        "https://2a3db3e9cab34ab2996414dd5bf6e169@o229745.ingest.sentry.io/5843753",
        sentry::ClientOptions {
            release: sentry::release_name!(),
            ..Default::default()
        },
    ));
    setup_db().await;

    client_update_check::check_updates().await;

    let mut manager = Manager::new();
    manager.start().await;
}

struct Manager {
    bridge_thread: Option<JoinHandle<()>>,
    state: Arc<Mutex<StateWrapper>>,
    sender: Sender<EventType>,
    receiver: Receiver<EventType>,
    websockets: Arc<tokio::sync::Mutex<HashMap<u128, WebSocketStream<Upgraded>>>>,
}

impl Manager {
    fn new() -> Self {
        let (sender, receiver) = unbounded();
        Self {
            bridge_thread: None,
            state: Arc::new(Mutex::new(StateWrapper {
                state: BridgeState::DISCONNECTED,
                description: StateDescription::None,
            })),
            sender,
            receiver,
            websockets: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        }
    }

    async fn start<'a>(&'a mut self) {
        let dist_sender_clone = self.sender.clone();
        let (bridge_sender, bridge_receiver) = unbounded();
        let websockets = self.websockets.clone();
        let stateinfo = self.state.clone();
        spawn(async move {
            let _ = spawn(ApiManager::start(dist_sender_clone, websockets, stateinfo));
        });
        self.connect_boot(self.sender.clone(), self.state.clone())
            .await;
        let websockets = self.websockets.clone();
        spawn(async move {
            loop {
                api_manager::websocket_handler::check_incoming_messages(websockets.clone())
                    .await;
                sleep(Duration::from_secs(1)).await;
            }
        });
        loop {
            if let Ok(event) = self.receiver.try_recv() {
                match event {
                    EventType::CreateBridge { address, port } => {
                        println!(
                            "[MAIN] Creating new bridge instance: {}:{}",
                            &address, &port
                        );
                        if self.bridge_thread.is_some() {
                            send(&self.sender, EventType::CreateBridgeError {
                            error: "Tried to create a new connection while already connected. Aborted all connections".to_string(),
                    });

                            continue;
                        }

                        let dist_sender_clone = self.sender.clone();
                        let bridge_receiver_clone = bridge_receiver.clone();
                        let bridge_sender_clone = bridge_sender.clone();
                        let state = self.state.clone();
                        self.bridge_thread = Some(spawn(async move {
                            let panic_sender_clone = dist_sender_clone.clone();
                            std::panic::set_hook(Box::new(move |e| {
                                println!("[BRIDGE][PANIC] {}", e);
                                let msg = format!("{}", e);
                                sentry::capture_message(&msg, sentry::Level::Error);
                                send(&panic_sender_clone, EventType::StateUpdate ( 
                                    StateWrapper {
                                    state: BridgeState::ERRORED,
                                    description: StateDescription::Error {
                                        message: "An internal error occurred\nCheck the logs for more info.".to_string()
                                    }
                                })
                            );
                        }));
                            let mut bridge = Bridge::new(
                                dist_sender_clone,
                                bridge_sender_clone,
                                bridge_receiver_clone,
                                address,
                                port,
                                state,
                            );
                            bridge.start().await;
                        }));
                    }
                    EventType::CreateBridgeError { error } => {
                        eprintln!("[BRIDGE][Error]: {} ", error);
                        send(&bridge_sender, EventType::KillBridge);
                        send(
                            &self.sender,
                            EventType::StateUpdate(StateWrapper {
                                state: BridgeState::ERRORED,
                                description: api_manager::models::StateDescription::Error {
                                    message: error,
                                },
                            }),
                        );

                        if let Some(handle) = &self.bridge_thread {
                            handle.abort();
                            self.bridge_thread = None;
                        } else {
                            panic!("Connection error when thread was already closed.");
                            // continue
                        }
                    }
                    EventType::StateUpdate(new_state) => {
                        {
                            let old = self.state.lock().await;
                            println!("[STATEUPDATE] {:?} => {:?}", old.state, new_state.state);
                        }
                        *self.state.lock().await = new_state.clone();
                        self.send_websockets_updated_state(new_state.clone()).await;
                        if new_state.state == BridgeState::DISCONNECTED
                            || new_state.state == BridgeState::ERRORED
                        {
                            send(&bridge_sender, EventType::KillBridge);
                            self.bridge_thread.take();
                        }
                    }

                    EventType::PrintEnd => send(
                        &bridge_sender,
                        EventType::PrintEnd,
                    ),
                    EventType::PrintStart(info) => {
                        if self.bridge_thread.is_none() {
                            continue;
                        }
                        send(
                            &bridge_sender,
                            EventType::PrintStart (info),
                        );
                    }
                    EventType::TempUpdate {
                        tools,
                        bed,
                        chamber,
                    } => {
                        let json = json!({
                                "type": "temperature_change",
                                "content": {
                                        "tools": tools,
                                        "bed": bed,
                                        "chamber": chamber,
                                        "time": Utc::now().timestamp_millis()
                                },
                        });
                        let mut delete_queue: Vec<u128> = vec![];
                        for sender in self.websockets.lock().await.iter_mut() {
                            let result = sender.1.send(Message::text(json.to_string())).await;

                            if result.is_err() {
                                println!(
                                    "[WS][ERROR] ID: {} | {}",
                                    Uuid::from_u128(sender.0.clone()).to_hyphenated(),
                                    result.unwrap_err()
                                );
                                delete_queue.push(sender.0.clone());
                            }
                        }
                        for id in delete_queue {
                            let mut guard = self.websockets.lock().await;
                            guard.remove(&id);
                        }
                    }

                    EventType::IncomingTerminalMessage(message) => {
                        let time: DateTime<Utc> = Utc::now();
                        let json = json!({
                                "type": "terminal_message",
                                "content": [
                                        {
                                                "message": message,
                                                "type": "OUTPUT",
                                                "id": null,
                                                "time": time.to_rfc3339()
                                        }
                                ]
                        });
                        for sender in self.websockets.lock().await.iter_mut() {
                            let result = sender.1.send(Message::text(json.to_string())).await;
                            if result.is_err() {
                                println!(
                                    "[WS] Connection closed: {}",
                                    Uuid::from_u128(sender.0.clone()).to_hyphenated()
                                );
                                self.websockets.lock().await.remove(sender.0);
                            }
                        }
                    }

                    EventType::OutGoingTerminalMessage(message) => {
                        let time: DateTime<Utc> = Utc::now();
                        let json = json!({
                                "type": "terminal_message",
                                "content": [
                                        {
                                                "message": message.content.trim(),
                                                "type": "INPUT",
                                                "id": message.id.to_hyphenated().to_string(),
                                                "time": time.to_rfc3339()
                                        }
                                ]
                        });
                        send(&bridge_sender, EventType::OutGoingTerminalMessage(message.clone()));
                        println!("[SENDING TO WS]");

                        for sender in self.websockets.lock().await.iter_mut() {
                            sender
                                .1
                                .send(Message::text(json.to_string()))
                                .await
                                .expect("Cannot send message");
                        }
                        
                    }

                    EventType::KillBridge => {
                        eprintln!("[WARNING] Received KillBridge event on main receiver");
                    }
                }
            } else {
                let time = Instant::now();
                yield_now().await;
                let state = self.state.lock().await.state;
                if state.ne(&BridgeState::PRINTING) && time.elapsed().as_millis() < 300 {
                    sleep(tokio::time::Duration::from_millis(
                        300 - time.elapsed().as_millis() as u64,
                    ))
                    .await;
                } else if state.eq(&BridgeState::PRINTING) && time.elapsed().as_millis() < 3 {
                    sleep(tokio::time::Duration::from_millis(
                        3 - time.elapsed().as_millis() as u64,
                    ))
                    .await;
                }
            }
        }
    }
    /*
        Fetch settings for automatically connecting on starting application (boot setting, device path & baudrate).
        Check if those settings are correctly set and autoboot is enabled.
        Try to send a connectionCreate event to the global dist sender.
    */
    async fn connect_boot(&self, sender: Sender<EventType>, state: Arc<Mutex<StateWrapper>>) {
        spawn(async move {
            if state.lock().await.state != BridgeState::DISCONNECTED {
                return;
            }

            let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
            let query = sqlx::query_as::<_, SettingRow>(
                "SELECT * FROM settings where id = 'B_startOnBoot' or id = 'S_devicePath' or id = 'N_deviceBaud'",
            );

            let result = query.fetch_all(&mut connection).await;
            if result.is_ok() {
                let rows = result.unwrap();
                if rows.len() == 3 {
                    let mut address: Option<String> = None;
                    let mut baud_rate = 0;
                    let mut connect = false;

                    for row in rows {
                        if row.id == "B_startOnBoot" {
                            connect = row.bool.unwrap();
                        } else if row.id == "S_devicePath" {
                            address = Some(row.raw_value);
                        } else if row.id == "N_deviceBaud" {
                            baud_rate = row.number.unwrap() as u32;
                        }
                    }
                    if connect && baud_rate != 0 && address.is_some() {
                        println!("[BRIDGE] Connect on boot is set, trying to connect.");
                        send(
                            &sender,
                            EventType::CreateBridge {
                                address: address.unwrap(),
                                port: baud_rate,
                            },
                        );
                    }
                }
            }
        });
    }

    async fn send_websockets_updated_state(&self, state_info: StateWrapper) {
        let json = match state_info.state {
            BridgeState::DISCONNECTED => json!({
                    "type": "state_update",
                    "content": {
                            "state": "Disconnected",
                            "description": serde_json::Value::Null
                    }
            })
            .to_string(),
            BridgeState::CONNECTING => json!({
                    "type": "state_update",
                    "content": {
                            "state": "Connecting",
                            "description": serde_json::Value::Null
                    }
            })
            .to_string(),
            BridgeState::CONNECTED => json!({
                    "type": "state_update",
                    "content": {
                            "state": "Connected",
                            "description": serde_json::Value::Null
                    }
            })
            .to_string(),
            BridgeState::ERRORED => match state_info.description {
                StateDescription::Error { message } => json!({
                        "type": "state_update",
                        "content": {
                                "state": "Errored",
                                "description": {
                                        "errorDescription": message
                                }
                        }
                })
                .to_string(),
                _ => json!({
                        "type": "state_update",
                        "content": {
                                "state": "Errored",
                                "description": serde_json::Value::Null
                        }
                })
                .to_string(),
            },
            BridgeState::PREPARING => todo!(),
            BridgeState::PRINTING => match state_info.description {
                StateDescription::Print {
                    filename,
                    progress,
                    start,
                    end,
                } => {
                    let mut end_string: Option<String> = None;
                    if end.is_some() {
                        end_string = Some(end.unwrap().to_rfc3339());
                    }
                    json!({
                            "type": "state_update",
                            "content": {
                                "state": "Printing",
                                "description": {
                                    "printInfo": {
                                        "file": {
                                            "name": filename,
                                        },
                                        "progress": format!("{:.2}", progress),
                                        "startTime": start.to_rfc3339(),
                                        "estEndTime": end_string
                                    }
                                }
                            }
                    })
                    .to_string()
                }
                _ => json!({
                        "type": "state_update",
                        "content": {
                                "state": "Printing",
                                "description": serde_json::Value::Null
                        }
                })
                .to_string(),
            },
            BridgeState::FINISHING => todo!(),
        };
        for sender in self.websockets.lock().await.iter_mut() {
            sender
                .1
                .send(Message::text(json.to_string()))
                .await
                .expect("Cannot send message");
        }
    }
}

async fn setup_db() {
    let _ = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open("storage.db")
        .await;

    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    connection
        .execute(
            "
        CREATE TABLE IF NOT EXISTS users (
            username VARCHAR(255) NOT NULL primary key,
            password VARCHAR(255) NOT NULL,
            permissions INTEGER NOT NULL
        );

        CREATE TABLE IF NOT EXISTS tokens (
            username VARCHAR(255) NOT NULL,
            token VARCHAR(255) NOT NULL primary key,
            expire DATETIME,
            FOREIGN KEY(username) REFERENCES users(username) on update cascade on delete cascade
        );
        
        CREATE TABLE IF NOT EXISTS settings (
            id varchar(255) primary key,
            value TEXT,
            type integer(3) not null
        );

        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('S_devicePath', 0, null);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('N_deviceBaud', 2, null);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('B_startOnBoot', 1, false);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('F_adjustCorrectionF', 3, null);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('B_savePrinterNotifications', 1, true);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('B_savePrinterNotifications', 1, true);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('N_deviceWidth', 2, null);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('N_deviceHeight', 2, null);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('N_deviceDepth', 2, null);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('B_deviceHB', 1, false);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('B_deviceHC', 1, false);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('N_clientTerminalAmount', 2, 500);
        INSERT OR IGNORE INTO SETTINGS (id, type, value) VALUES ('S_sentryDsn', 0, 'https://cd35379ff0fc45daa30a67bfe9aa8b36@0229745.ingest.sentry.io/5778789');

        DELETE FROM tokens where expire < DATE('now');
    ",
        )
        .await
        .expect("Error while creating tables.");
}
