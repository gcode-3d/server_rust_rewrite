use futures::{FutureExt, SinkExt, StreamExt};
use hyper::upgrade::Upgraded;
use hyper_tungstenite::tungstenite::{
    protocol::{frame::coding::CloseCode, CloseFrame},
    Message,
};
use hyper_tungstenite::WebSocketStream;
use serde_json::{json, Value};
use std::{collections::HashMap, sync::Arc};
use tokio::{sync::Mutex, task::yield_now};
use uuid::Uuid;

use crate::api_manager::models::{self, BridgeState};

use super::models::{AuthPermissions, StateWrapper};
/*
    Function gets called by the router after the request has been upgraded to a websocket connection.
    The function keeps loaded as long as a connection is created


    Arguments:
    - websocket: The websocket object
    - user: parsed user including permissions.
    - receiver: Global receiver to catch events related to websockets.
    - state: current state arc, used for sending intial ready event.
    - sockets: hashmap including all websocket senders, mapped by uuid.

*/
pub async fn handler(
    websocket: WebSocketStream<Upgraded>,
    user: AuthPermissions,
    state: Arc<Mutex<StateWrapper>>,
    sockets: Arc<Mutex<HashMap<u128, WebSocketStream<Upgraded>>>>,
) -> Result<(), Box<dyn std::error::Error>> {
    let id = Uuid::new_v4();
    {
        sockets.lock().await.insert(id.clone().as_u128(), websocket);

        println!(
            "[WS][CONNECTION] ID: {} | User: {}",
            id.to_hyphenated(),
            &user.username()
        );
    }

    /*
            Construct intial ready event message.
    */
    let mut json = json!({
                    "type":"ready",
                    "content": {}
    });
    let content = json.get_mut("content").unwrap();
    let user = json!({
    "username": user.username(),
    "permissions" : {
             "admin": user.admin() ,
             "connection.edit": user.edit_connection(),
             "file.access": user.file_access(),
             "file.edit": user.file_edit(),
             "print_state.edit": user.print_state_edit(),
             "settings.edit": user.settings_edit(),
             "permissions.edit": user.users_edit(),
             "terminal.read": user.terminal_read(),
             "terminal.send": user.terminal_send(),
             "webcam.view": user.webcam(),
             "update.check": user.update(),
             "update.manage": user.update()
            }
    });
    let state_info = state.lock().await;
    let state_info = state_info.clone();

    match state_info.state {
        BridgeState::DISCONNECTED => {
            *content = json!({
                    "user": user,
                    "state": "Disconnected",
            });
        }
        BridgeState::CONNECTING => {
            *content = json!({
                    "user": user,
                    "state": "Connecting",
            });
        }
        BridgeState::CONNECTED => {
            *content = json!({
                    "user": user,
                    "state": "Connected",
            });
        }
        BridgeState::ERRORED => {
            let description = match state_info.description.clone() {
                models::StateDescription::Error { message } => message,
                _ => "Unknown error".to_string(),
            };

            *content = json!({
                    "user": user,
                    "state": "Errored",
                    "description": {
                            "errorDescription": description
                    }
            });
        }
        BridgeState::PREPARING => todo!(),
        BridgeState::PRINTING => {
            let description = match state_info.description.clone() {
                models::StateDescription::Print {
                    filename,
                    progress,
                    start,
                    end,
                } => {
                    let mut end_string = None;
                    if end.is_some() {
                        end_string = Some(end.unwrap().to_rfc3339());
                    }
                    json!({
                            "printInfo": {
                                    "file": {
                                            "name": filename,
                                    },
                            "progress": format!("{:.2}", progress),
                            "startTime": start.to_rfc3339(),
                            "estEndTime": end_string
                    }})
                }
                _ => Value::Null,
            };
            *content = json!({
                    "user": user,
                    "state": "Printing",
                    "description": description
            });
        }
        BridgeState::FINISHING => todo!(),
    };
    let mut guard = sockets.lock().await;
    let socket = guard.get_mut(&id.as_u128());
    if socket.is_some() {
        let socket = socket.unwrap();

        socket
            .send(Message::text(json.to_string()))
            .await
            .expect("Cannot send message");
    }
    return Ok(());
}

pub async fn check_incoming_messages(
    sockets: Arc<Mutex<HashMap<u128, WebSocketStream<Upgraded>>>>,
) {
    let mut delete_queue: Vec<u128> = vec![];
    {
        for socket in sockets.lock().await.iter_mut() {
            let id = socket.0;
            let socket = socket.1;
            if let Some(result) = socket.next().now_or_never() {
                if result.is_none() {
                    yield_now().await;
                    continue;
                }
                let result = result.unwrap();

                match result {
                    Ok(message) => {
                        if message.is_close() {
                            delete_queue.push(id.clone());
                            continue;
                        }
                        if message.is_ping() {
                            socket
                                .send(Message::Pong(message.into_data()))
                                .await
                                .expect("Cannot send message");
                            continue;
                        }

                        close_socket(id, socket, CloseCode::Unsupported).await;
                    }
                    Err(e) => {
                        eprintln!("[ERROR][WS] {}", e);
                        delete_queue.push(id.clone());
                    }
                }
            }
        }
    }

    let mut sockets = sockets.lock().await;
    for id in delete_queue {
        {
            let socket = sockets.get_mut(&id);
            if socket.is_none() {
                continue;
            }
            let socket = socket.unwrap();
            close_socket(&id, socket, CloseCode::Normal).await;
        }
        sockets.remove(&id);
    }
}

async fn close_socket(id: &u128, socket: &mut WebSocketStream<Upgraded>, close_code: CloseCode) {
    println!("[WS][DISCONNECT] ID: {}", &id);
    let _ = socket
        .send(Message::Close(Some(CloseFrame {
            code: close_code,
            reason: std::borrow::Cow::Borrowed(""),
        })))
        .await;
}

pub async fn send_to_all_ws_clients(
    message: String,
    sockets: &Arc<Mutex<HashMap<u128, WebSocketStream<Upgraded>>>>,
) {
    print!("x1");
    let mut delete_queue: Vec<u128> = vec![];
    {
        if sockets.lock().await.len() == 0 {
            print!("x2");
            return;
        }
    }
    {
        print!("x3");
        for socket in sockets.lock().await.iter_mut() {
            print!("x4");
            let id = socket.0;
            let socket = socket.1;

            let result = socket.send(Message::Text(message.clone())).await;

            if result.is_err() {
                eprintln!(
                    "[WS][ERROR] ID: {} | {}",
                    Uuid::from_u128(*id).to_hyphenated(),
                    result.unwrap_err()
                );
                delete_queue.push(*id);
            }
        }
    }
    print!("x5");

    if delete_queue.len() > 0 {
        print!("x6");

        for socket in delete_queue {
            print!("x7");
            sockets.lock().await.remove(&socket);
        }
    }
}
