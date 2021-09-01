/*
    Send a event to the bridge to start creating connection.

    PUT /api/connection

    Permission: connection.edit
    State: Disconnected | Errored
*/

use crossbeam_channel::Sender;
use hyper::{header, Body, Request, Response};
use sqlx::{Connection, SqliteConnection};

use crate::{
    api_manager::{
        models::{BridgeEvents, EventInfo, EventType, SettingRow, StateWrapper},
        responses::forbidden_response,
    },
    bridge::BridgeState,
};
pub const METHODS: &str = "PUT, DELETE, POST";
pub const PATH: &str = "/api/connection";

pub async fn handler(
    _request: Request<Body>,
    distributor: Sender<EventInfo>,
    state_info: StateWrapper,
) -> Response<Body> {
    if !(state_info.state == BridgeState::DISCONNECTED || state_info.state == BridgeState::ERRORED)
    {
        return forbidden_response();
    }

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
            .header(header::ACCESS_CONTROL_ALLOW_METHODS, "PUT")
            .status(400)
            .body(Body::from("Bad Request"))
            .expect("Failed to construct valid response");
    }

    distributor
        .send(EventInfo {
            event_type: EventType::Bridge(BridgeEvents::ConnectionCreate {
                address: result.clone().unwrap().address,
                port: result.unwrap().port,
            }),
        })
        .expect("Cannot send message");

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
