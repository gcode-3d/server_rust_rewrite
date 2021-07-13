use crossbeam_channel::{Receiver, Sender};
use serialport;

use crate::{
    api_manager::{
        self,
        models::{
            BridgeEvents, EventInfo,
            EventType::{self},
            WebsocketEvents,
        },
    },
    bridge,
};

pub struct Bridge {
    address: String,
    baudrate: u32,
    distibutor: Sender<EventInfo>,
    receiver: Receiver<EventInfo>,
}

impl Bridge {
    pub fn new(
        distibutor: Sender<EventInfo>,
        receiver: Receiver<EventInfo>,
        address: String,
        baudrate: u32,
    ) -> Self {
        println!("[BRIDGE] Created new Bridge instance");
        return Self {
            address,
            baudrate,
            distibutor,
            receiver,
        };
    }

    // if port fails, emit failure message to distributor.
    pub fn start(&self) {
        let _ = self.distibutor.send(EventInfo {
            event_type: EventType::Websocket(WebsocketEvents::StateUpdate {
                state: api_manager::models::State::Connecting,
            }),
            message_data: "".to_string(),
        });
        match serialport::new(&self.address, self.baudrate).open() {
            Ok(mut port) => {
                println!("[BRIDGE] Port opened");
                let _ = self.distibutor.send(EventInfo {
                    event_type: EventType::Websocket(WebsocketEvents::StateUpdate {
                        state: api_manager::models::State::Connected,
                    }),
                    message_data: "".to_string(),
                });

                let result = port.write(b"G28\n");
                if result.is_ok() {
                    println!("[BRIDGE] Write result: {}", result.unwrap());
                    port.flush().expect("FLUSH FAIL")
                } else {
                    eprintln!("[BRIDGE] Write errored: {}", result.unwrap_err())
                }
            }
            Err(err) => match err.kind {
                serialport::ErrorKind::NoDevice => {
                    let _ = self.distibutor.send(EventInfo {
                        event_type: EventType::Bridge(
                            bridge::BridgeEvents::ConnectionCreateError { error: 0 },
                        ),
                        message_data: "".to_string(),
                    });
                }
                serialport::ErrorKind::InvalidInput => todo!("INV. INPUT"),
                serialport::ErrorKind::Unknown => todo!("??"),
                serialport::ErrorKind::Io(_) => todo!("IO ERROR"),
            },
        };
    }
}
