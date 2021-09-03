/*
    Sends a disconnect, and after a second a connect attempt event to the bridge.

    POST /api/connection


    Permission: print_state.edit
    State: Connected | Printing
*/

use std::time::Duration;

use crossbeam_channel::Sender;
use hyper::{header, Body, Response};
use serde_json::json;
use sqlx::{Connection, SqliteConnection};
use tokio::time::sleep;

use crate::{
    api_manager::models::{send, BridgeEvents, EventInfo, EventType, SettingRow, StateDescription},
    bridge::BridgeState,
};

pub const METHODS: &str = "PUT, DELETE, POST";
pub const PATH: &str = "/api/connection";

pub async fn handler(state: BridgeState, distributor: Sender<EventInfo>) -> Response<Body> {
    if state.eq(&BridgeState::DISCONNECTED) || state.eq(&BridgeState::ERRORED) {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/plain")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "PUT, DELETE")
            .status(403)
            .body(Body::from(
                json!({"error": true, "message": "Not connected"}).to_string(),
            ))
            .expect("Failed to construct valid response");
    }

    distributor
        .send(EventInfo {
            event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                state: BridgeState::DISCONNECTED,
                description: StateDescription::None,
            }),
        })
        .expect("Cannot send message");
    sleep(Duration::from_millis(100)).await;

    let result = async {
        let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
        let query = sqlx::query_as::<_, SettingRow>("select * from settings");
        match query.fetch_all(&mut connection).await {
            Ok(settings) => {
                let mut address: Option<String> = None;
                let mut port: Option<u32> = None;

                for setting in settings.iter() {
                    if setting.id == "S_devicePath" {
                        address = Some(setting.raw_value.clone())
                    }
                    if setting.id == "N_deviceBaud" {
                        port = Some(setting.number.unwrap().clone() as u32);
                    }
                }
                if address.is_none() || port.is_none() {
                    eprintln!("[API][ERROR] No address / port set up");
                    return None;
                }
                if address.clone().unwrap().len() == 0 || port.clone().unwrap() == 0 {
                    eprintln!("[API][ERROR] No address / port set up");
                    return None;
                }
                return Some(ConnectionInfo::new(address.unwrap(), port.unwrap()));
            }
            Err(err) => {
                eprintln!("[API][ERROR] {}", err);
                return None;
            }
        }
    }
    .await;
    if result.is_none() {
        return Response::builder()
            .header(header::CONTENT_TYPE, "text/plain")
            .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
            .status(400)
            .body(Body::from("Bad Request"))
            .expect("Failed to construct valid response");
    }

    send(
        &distributor,
        EventType::Bridge(BridgeEvents::ConnectionCreate {
            address: result.clone().unwrap().address,
            port: result.unwrap().port,
        }),
    );

    return Response::builder()
        .header(header::CONTENT_TYPE, "text/plain")
        .header(header::ACCESS_CONTROL_ALLOW_ORIGIN, "*")
        .header(header::ACCESS_CONTROL_ALLOW_METHODS, METHODS)
        .body(Body::empty())
        .expect("Failed to construct valid response");
}

#[derive(Debug, Clone)]
struct ConnectionInfo {
    pub address: String,
    pub port: u32,
}

impl ConnectionInfo {
    fn new(address: String, port: u32) -> Self {
        Self { address, port }
    }
}
