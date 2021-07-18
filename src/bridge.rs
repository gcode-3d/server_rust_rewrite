use std::{io::Write, time::Duration};

use crossbeam_channel::{Receiver, Sender};
use serialport;
use tokio::spawn;

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

pub struct Bridge {
    address: String,
    baudrate: u32,
    pub state: BridgeState,
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
            state: BridgeState::DISCONNECTED,
            distibutor,
            receiver,
        };
    }

    // if port fails, emit failure message to distributor.
    pub fn start(&mut self) {
        let _ = self.distibutor.send(EventInfo {
            event_type: EventType::Websocket(WebsocketEvents::StateUpdate {
                state: api_manager::models::State::Connecting,
            }),
            message_data: "".to_string(),
        });
        match serialport::new(&self.address, self.baudrate).open() {
            Ok(mut port) => {
                println!("[BRIDGE] Port opened");
                self.state = BridgeState::CONNECTED;
                let _ = self.distibutor.send(EventInfo {
                    event_type: EventType::Websocket(WebsocketEvents::StateUpdate {
                        state: api_manager::models::State::Connected,
                    }),
                    message_data: "".to_string(),
                });
                port.set_timeout(Duration::from_millis(10))
                    .expect("Cannot set timeout on port");
                let mut incoming = port;
                let mut outgoing = incoming.try_clone().expect("Cannot clone serialport");
                let receiver = self.receiver.clone();
                spawn(async move {
                    while let Some(message) = receiver.iter().next() {
                        println!("{:?}", message);
                    }
                });

                let distributor = self.distibutor.clone();
                spawn(async move {
                    let mut serial_buf: Vec<u8> = vec![0; 1];
                    let mut collected = String::new();
                    let mut is_finished_receiving_cap = false;
                    let mut cap_data: Vec<String> = vec![];
                    let cloned_dist = distributor.clone();
                    loop {
                        match incoming.read(serial_buf.as_mut_slice()) {
                            Ok(t) => {
                                let data = String::from_utf8_lossy(&serial_buf[..t]);
                                let string = data.into_owned();
                                if string == "\n" {
                                    if is_finished_receiving_cap {
                                        let _ = distributor.send(EventInfo {
                                            event_type: EventType::Websocket(
                                                WebsocketEvents::TerminalRead {
                                                    message: collected.clone(),
                                                },
                                            ),
                                            message_data: "".to_string(),
                                        });
                                    } else {
                                        if collected.starts_with("ok") {
                                            if cap_data.len() == 0 {
                                                continue;
                                            }
                                            if !cap_data[0].starts_with("FIRMWARE_NAME:Marlin") {
                                                cap_data = vec![];
                                                incoming
                                                    .write(b"M115\n")
                                                    .expect("Cannot resend M115.");
                                            } else {
                                                is_finished_receiving_cap = true;
                                                for cap in &cap_data {
                                                    println!("[BRIDGE][CAP] => {}", cap);
                                                }
                                                cap_data.push(collected);
                                            }
                                        } else {
                                            cap_data.push(collected);
                                        }
                                    }

                                    collected = String::new();
                                } else {
                                    for char in string.chars() {
                                        collected.push(char);
                                    }
                                }
                            }
                            Err(ref e) => match e.kind() {
                                std::io::ErrorKind::TimedOut => (),
                                _ => {
                                    eprintln!("[BRIDGE] Read error: {:?}", e);
                                    let _ = cloned_dist.send(EventInfo {
                                        event_type: EventType::Websocket(
                                            WebsocketEvents::StateUpdate {
                                                state: api_manager::models::State::Errored {
                                                    description: format!("{}", e),
                                                },
                                            },
                                        ),
                                        message_data: "".to_string(),
                                    });
                                    break;
                                }
                            },
                        }
                    }
                });
                let result = outgoing.write(b"M155 S2\n");
                if result.is_ok() {
                    println!("[BRIDGE] Write result: {}", result.unwrap());
                    outgoing.flush().expect("FLUSH FAIL");
                } else {
                    eprintln!("[BRIDGE] Write errored: {}", result.unwrap_err())
                }
            }
            Err(err) => match err.kind {
                serialport::ErrorKind::NoDevice => {
                    let _ = self.distibutor.send(EventInfo {
                        event_type: EventType::Bridge(
                            bridge::BridgeEvents::ConnectionCreateError {
                                error: err.description,
                            },
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
