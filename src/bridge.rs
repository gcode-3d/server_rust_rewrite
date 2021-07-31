use std::{
    io::Write,
    sync::{Arc, Mutex},
    thread::sleep,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender};
use serialport;
use tokio::{spawn, task};

use crate::{
    api_manager::{
        self,
        models::{
            BridgeEvents, EventInfo,
            EventType::{self},
            PrintInfo, WebsocketEvents,
        },
    },
    bridge, parser,
};

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    pub state: Arc<Mutex<BridgeState>>,
    print_info: Arc<Mutex<Option<PrintInfo>>>,
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
            state: Arc::new(Mutex::new(BridgeState::DISCONNECTED)),
            print_info: Arc::new(Mutex::new(None)),
            distibutor,
            receiver,
        };
    }

    // if port fails, emit failure message to distributor.
    pub fn start(&mut self) {
        let is_canceled = Arc::new(Mutex::new(false));
        *self.state.lock().unwrap() = BridgeState::CONNECTING;
        self.distibutor
            .send(EventInfo {
                event_type: EventType::Websocket(WebsocketEvents::StateUpdate {
                    state: BridgeState::CONNECTING,
                    description: api_manager::models::StateDescription::None,
                }),
            })
            .expect("cannot send message");

        let port_result = serialport::new(&self.address, self.baudrate).open();
        if port_result.is_err() {
            let err = port_result.err().unwrap();
            match err.kind {
                serialport::ErrorKind::NoDevice => {
                    self.distibutor
                        .send(EventInfo {
                            event_type: EventType::Bridge(
                                bridge::BridgeEvents::ConnectionCreateError {
                                    error: err.description,
                                },
                            ),
                        })
                        .expect("cannot send message");
                }
                serialport::ErrorKind::InvalidInput => todo!("INV. INPUT"),
                serialport::ErrorKind::Unknown => todo!("??"),
                serialport::ErrorKind::Io(_) => todo!("IO ERROR"),
            }
            return;
        }
        let distributor = self.distibutor.clone();
        let state = self.state.clone();
        task::spawn_blocking(|| async move {
            sleep(Duration::from_secs(10));
            if *state.lock().unwrap() == BridgeState::CONNECTING {
                distributor
                    .send(EventInfo {
                        event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                            state: BridgeState::ERRORED,
                            description: api_manager::models::StateDescription::Error {
                                message: "Timed out".to_string(),
                            },
                        }),
                    })
                    .expect("Cannot send message");
            }
        });

        let mut port = port_result.unwrap();

        port.set_timeout(Duration::from_millis(10))
            .expect("Cannot set timeout on port");
        let mut incoming = port;
        let mut outgoing = incoming.try_clone().expect("Cannot clone serialport");
        let outgoing_for_listener = incoming.try_clone().expect("Cannot clone serialport");
        let receiver = self.receiver.clone();
        let print_info = self.print_info.clone();
        let distributor = self.distibutor.clone();
        let current_state = self.state.clone();
        let canceled = is_canceled.clone();
        task::spawn_blocking(|| async move {
            let mut outgoing = outgoing_for_listener;
            println!(
                "[Bridge] connected to port {} with {} baudrate",
                outgoing.as_ref().name().unwrap_or("UNNAMED".to_string()),
                outgoing.as_ref().baud_rate().unwrap()
            );

            while let Some(event) = receiver.iter().next() {
                match event.event_type {
                    EventType::KILL => {
                        drop(outgoing);
                        *canceled.lock().unwrap() = true;
                        break;
                    }
                    EventType::Bridge(BridgeEvents::TerminalSend { mut message }) => {
                        if !message.ends_with("\n") {
                            message = format!("{}\n", message);
                        }
                        let result = outgoing.write(message.as_bytes());
                        if result.is_err() {
                            let err = result.unwrap_err();
                            eprintln!("[BRIDGE][ERROR] {}", err);
                            distributor
                                .send(EventInfo {
                                    event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                                        state: BridgeState::ERRORED,
                                        description: api_manager::models::StateDescription::Error {
                                            message: err.to_string(),
                                        },
                                    }),
                                })
                                .expect("Cannot send message");
                        }
                    }
                    EventType::Bridge(BridgeEvents::TerminalRead { message: _ }) => {}
                    EventType::Bridge(BridgeEvents::PrintEnd) => {
                        if current_state.lock().unwrap().ne(&BridgeState::PRINTING) {
                            return ();
                        }
                        *print_info.lock().unwrap() = None;
                        distributor
                            .send(EventInfo {
                                event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                                    state: BridgeState::CONNECTED,
                                    description: api_manager::models::StateDescription::None,
                                }),
                            })
                            .expect("Cannot send message");
                    }
                    EventType::Bridge(BridgeEvents::PrintStart { info }) => {
                        if current_state.lock().unwrap().ne(&BridgeState::CONNECTED) {
                            return ();
                        }
                        let mut guard = print_info.lock().unwrap();
                        let filename = info.filename.clone();
                        let progress = info.progress();
                        let start = info.start.clone();
                        let end = info.end.clone();

                        *guard = Some(info);
                        distributor
                            .send(EventInfo {
                                event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                                    state: BridgeState::PRINTING,
                                    description: api_manager::models::StateDescription::Print {
                                        filename,
                                        progress,
                                        start,
                                        end,
                                    },
                                }),
                            })
                            .expect("Cannot send message");

                        distributor
                            .send(EventInfo {
                                event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                                    message: "M110 N0".to_string(),
                                }),
                            })
                            .expect("Cannot send first line");
                    }
                    EventType::Bridge(BridgeEvents::StateUpdate {
                        state,
                        description: _,
                    }) => {
                        *current_state.lock().unwrap() = state;
                    }

                    _ => (),
                }
            }
        });

        let distributor = self.distibutor.clone();
        let print_info = self.print_info.clone();
        let state = self.state.clone();
        let canceled = is_canceled.clone();
        task::spawn_blocking(|| async move {
            let mut serial_buf: Vec<u8> = vec![0; 1];
            let mut collected = String::new();
            let mut cap_data: Vec<String> = vec![];
            let mut has_collected_capabilities = false;
            let mut commands_left_to_send: Vec<String> = vec![];

            let cloned_dist = distributor.clone();
            loop {
                match incoming.read(serial_buf.as_mut_slice()) {
                    Ok(t) => {
                        if *canceled.lock().unwrap() {
                            break;
                        }
                        let data = String::from_utf8_lossy(&serial_buf[..t]);
                        let string = data.into_owned();
                        if string == "\n" {
                            println!("=> {}", collected);

                            if state.lock().unwrap().eq(&BridgeState::CONNECTING) {
                                if collected.to_lowercase().starts_with("error") {
                                    parser::parse_line(
                                        &distributor,
                                        &collected,
                                        state.lock().unwrap().clone(),
                                        print_info.clone(),
                                    );
                                    break;
                                }
                                if has_collected_capabilities && collected.starts_with("ok") {
                                    parser::parse_line(
                                        &distributor,
                                        &collected,
                                        state.lock().unwrap().clone(),
                                        print_info.clone(),
                                    );
                                    collected = String::new();

                                    if commands_left_to_send.len() == 0 {
                                        *state.lock().unwrap() = BridgeState::CONNECTED;

                                        distributor
                                            .send(EventInfo {
                                                event_type: EventType::Bridge(
                                                    BridgeEvents::StateUpdate {
                                                        state: BridgeState::CONNECTED,
                                                        description: api_manager::models::StateDescription::Capability {
                                                            capabilities: cap_data.clone()
                                                        },
                                                    },
                                                ),
                                            })
                                            .expect("Cannot send state update message");
                                        continue;
                                    }

                                    let command = commands_left_to_send.pop().unwrap();
                                    distributor
                                        .send(EventInfo {
                                            event_type: EventType::Bridge(
                                                BridgeEvents::TerminalSend {
                                                    message: command.clone(),
                                                },
                                            ),
                                        })
                                        .expect("Cannot send websocket message");

                                    continue;
                                }
                                if collected.starts_with("ok") {
                                    if cap_data.len() == 0 {
                                        continue;
                                    }
                                    if !cap_data[0].starts_with("FIRMWARE_NAME:Marlin") {
                                        cap_data = vec![];
                                        incoming.write(b"M115\n").expect("Cannot resend M115.");
                                    } else {
                                        for cap in &cap_data {
                                            // println!("[BRIDGE][CAP] => {}", cap);

                                            if cap.contains("Cap:AUTOREPORT_TEMP:1") {
                                                commands_left_to_send.push("M155 S2".to_string());
                                            } else {
                                                // TODO: ADD TEMP REPORTING.
                                                // let is_canceled = canceled.clone();
                                                // spawn(async move {
                                                //     loop {
                                                //         if *is_canceled.lock().unwrap() {
                                                //             break;
                                                //         }
                                                //         std::thread::sleep(Duration::from_secs(2));
                                                //     }
                                                // });
                                            }
                                            if cap.contains("Cap:EEPROM:1") {
                                                commands_left_to_send.push("M501".to_string())
                                            }
                                        }
                                        distributor
                                            .send(EventInfo {
                                                event_type: EventType::Bridge(
                                                    BridgeEvents::TerminalSend {
                                                        message: "G90".to_string(),
                                                    },
                                                ),
                                            })
                                            .expect("Cannot send websocket message");

                                        cap_data.push(collected);
                                        has_collected_capabilities = true;
                                    }
                                } else {
                                    cap_data.push(collected);
                                }
                            } else {
                                parser::parse_line(
                                    &distributor,
                                    &collected,
                                    state.lock().unwrap().clone(),
                                    print_info.clone(),
                                );
                            }
                            collected = String::new();
                        } else {
                            for char in string.chars() {
                                collected.push(char);
                            }
                        }
                    }
                    Err(ref e) => match e.kind() {
                        std::io::ErrorKind::TimedOut => {
                            if *is_canceled.lock().unwrap() {
                                break;
                            }
                        }
                        _ => {
                            if *is_canceled.lock().unwrap() {
                                break;
                            }
                            eprintln!("[BRIDGE][ERROR][READ]: {:?}", e);
                            cloned_dist
                                .send(EventInfo {
                                    event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                                        state: BridgeState::ERRORED,
                                        description: api_manager::models::StateDescription::Error {
                                            message: e.to_string(),
                                        },
                                    }),
                                })
                                .expect("cannot send message");
                            break;
                        }
                    },
                }
            }
        });
        let result = outgoing.write(b"M115\n");
        if result.is_ok() {
            outgoing.flush().expect("FLUSH FAIL");
        } else {
            eprintln!("[BRIDGE] Write errored: {}", result.unwrap_err())
        }
    }
}
