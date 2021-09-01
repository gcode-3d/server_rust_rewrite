use crossbeam_channel::{Receiver, Sender};
use lazy_static::lazy_static;
use regex::Regex;
use serialport::{self, SerialPort};
use std::{collections::VecDeque, io::Write, sync::Arc, time::Duration};
use tokio::{spawn, sync::Mutex, task::yield_now, time::sleep};
use uuid::Uuid;

use crate::{
    api_manager::{
        self,
        models::{
            BridgeAction, BridgeEvents, EventInfo,
            EventType::{self},
            Message, PrintInfo, StateDescription, StateWrapper, WebsocketEvents,
        },
    },
    bridge,
    parser::Parser,
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
    state: Arc<Mutex<StateWrapper>>,
    print_info: Arc<Mutex<Option<PrintInfo>>>,
    distributor: Sender<EventInfo>,
    sender: Sender<EventInfo>,
    receiver: Receiver<EventInfo>,
    message_queue: Arc<Mutex<VecDeque<Message>>>,
    ready: Arc<Mutex<bool>>,
}

lazy_static! {
    static ref TOOLTEMPREGEX: Regex = Regex::new(r"((T\d?):([\d\.]+) ?/([\d\.]+))+").unwrap();
}
impl Bridge {
    pub fn new(
        distibutor: Sender<EventInfo>,
        sender: Sender<EventInfo>,
        receiver: Receiver<EventInfo>,
        address: String,
        baudrate: u32,
        state: Arc<Mutex<StateWrapper>>,
    ) -> Self {
        println!("[BRIDGE] Created new Bridge instance");
        return Self {
            address,
            baudrate,
            state,
            print_info: Arc::new(Mutex::new(None)),
            distributor: distibutor,
            sender,
            receiver,
            message_queue: Arc::new(Mutex::new(VecDeque::new())),
            ready: Arc::new(Mutex::new(true)),
        };
    }

    // if port fails, emit failure message to distributor.
    pub async fn start(&mut self) {
        let is_canceled = Arc::new(Mutex::new(false));
        *self.state.lock().await = StateWrapper {
            state: BridgeState::CONNECTING,
            description: StateDescription::None,
        };

        self.distributor
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
                    self.distributor
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
                serialport::ErrorKind::Unknown => {
                    self.distributor
                        .send(EventInfo {
                            event_type: EventType::Bridge(
                                bridge::BridgeEvents::ConnectionCreateError {
                                    error: err.description,
                                },
                            ),
                        })
                        .expect("cannot send message");
                }

                serialport::ErrorKind::Io(_) => {
                    self.distributor
                        .send(EventInfo {
                            event_type: EventType::Bridge(
                                bridge::BridgeEvents::ConnectionCreateError {
                                    error: err.description,
                                },
                            ),
                        })
                        .expect("cannot send message");
                }
            }
            return;
        }

        Bridge::spawn_timeout(10, self.distributor.clone(), self.state.clone());

        let mut port = port_result.unwrap();

        port.set_timeout(Duration::from_millis(10))
            .expect("Cannot set timeout on port");
        let incoming = port;
        let mut outgoing = incoming.try_clone().expect("Cannot clone serialport");
        Bridge::spawn_event_listener(
            incoming.try_clone().expect("Cannot clone serialport"),
            self.receiver.clone(),
            self.distributor.clone(),
            self.print_info.clone(),
            self.state.clone(),
            is_canceled.clone(),
            self.message_queue.clone(),
            self.ready.clone(),
        );
        Bridge::spawn_bridge_serial_reader(
            self.distributor.clone(),
            self.sender.clone(),
            self.print_info.clone(),
            self.state.clone(),
            is_canceled.clone(),
            self.message_queue.clone(),
            self.ready.clone(),
            incoming,
        );

        let result = outgoing.write(b"M115\n");
        if result.is_ok() {
            outgoing.flush().expect("FLUSH FAIL");
        } else {
            eprintln!("[BRIDGE] Write errored: {}", result.unwrap_err())
        }
    }

    async fn handle_ok_response(
        distributor: &Sender<EventInfo>,
        bridge_sender: &Sender<EventInfo>,
        collected_responses: &Mutex<Vec<String>>,
        collected: &mut String,
        state: &Mutex<StateWrapper>,
        print_info: &Mutex<Option<PrintInfo>>,
        queue: &Mutex<VecDeque<Message>>,
        ready: &Mutex<bool>,
    ) {
        let action = Parser::parse_responses(collected_responses.lock().await.clone());
        *collected_responses.lock().await = vec![];
        *collected = "".to_string();
        match action {
            BridgeAction::Continue(line_number) => {
                let state = state.lock().await.state;
                if state.eq(&BridgeState::PRINTING) {
                    let mut guard = print_info.lock().await;
                    if guard.is_none() {
                        return;
                    }
                    let print_info = guard.as_mut().unwrap();
                    let line;
                    if line_number.is_some() {
                        let line_number = line_number.unwrap();
                        line = print_info.get_line_by_index(line_number + 1);
                        print_info.set_line_number(line_number);
                    } else if print_info.line_number() == 0 {
                        line = print_info.get_line_by_index(1);
                        print_info.set_line_number(1);
                    } else {
                        // skip line as it's probably just some unrelated echo without line nr.
                        return;
                    }
                    if line.is_none() {
                        distributor
                            .send(EventInfo {
                                event_type: EventType::Bridge(BridgeEvents::PrintEnd),
                            })
                            .expect("Cannot send message");
                        return;
                    }
                    let line = line.unwrap();
                    let prev_progress = format!("{:.1}", print_info.progress());

                    print_info.add_bytes_sent(line.content().len() as u64);
                    let difference = format!("{:.1}", print_info.progress())
                        .parse::<f64>()
                        .unwrap()
                        - prev_progress.parse::<f64>().unwrap();

                    if difference > 0.1 {
                        distributor
                            .send(EventInfo {
                                event_type: EventType::Websocket(WebsocketEvents::StateUpdate {
                                    state: BridgeState::PRINTING,
                                    description:
                                        crate::api_manager::models::StateDescription::Print {
                                            filename: print_info.filename.to_string(),
                                            progress: print_info.progress(),
                                            start: print_info.start,
                                            end: print_info.end,
                                        },
                                }),
                            })
                            .expect("Cannot send progress update");
                    }
                    bridge_sender
                        .send(EventInfo {
                            event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                                message: Parser::add_checksum(line.line_number(), line.content()),
                                id: Uuid::new_v4(),
                            }),
                        })
                        .expect("Cannot send message");
                    return;
                } else if state.eq(&BridgeState::CONNECTED) {
                    let message = queue.lock().await.pop_front();

                    if message.is_some() {
                        let message = message.unwrap();
                        bridge_sender
                            .send(EventInfo {
                                event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                                    message: message.content,
                                    id: message.id,
                                }),
                            })
                            .expect("Cannot send message");
                    } else {
                        *ready.lock().await = true;
                    }
                }
            }

            BridgeAction::Error => {
                distributor
                    .send(EventInfo {
                        event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                            state: BridgeState::ERRORED,
                            description: StateDescription::Error {
                                message:
                                    "Bridge encountered unknown error.\n See terminal for more info"
                                        .to_string(),
                            },
                        }),
                    })
                    .expect("Cannot send message");
            }

            BridgeAction::Resend(line_number) => {
                if state.lock().await.state.eq(&BridgeState::PRINTING) {
                    let mut guard = print_info.lock().await;
                    if guard.is_none() {
                        return;
                    }
                    let print_info = guard.as_mut().unwrap();
                    print_info.report_resend();
                    if print_info.get_resend_ratio() > 0.1 {
                        return distributor.send(EventInfo {
                            // TODO: replace this with a notification / setting to ignore this.
                            event_type: EventType::Bridge(
                                BridgeEvents::StateUpdate {
                                    state: BridgeState::ERRORED,
                                    description: StateDescription::Error {message: "Resend ratio went above 10%.\n Consider checking your connection".to_string()},
                                },
                            ),
                        }).expect("Cannot send message");
                    }
                    let line = print_info.get_line_by_index(line_number);
                    print_info.set_line_number(line_number);
                    if line.is_some() {
                        let line = line.unwrap();
                        bridge_sender
                            .send(EventInfo {
                                event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                                    message: Parser::add_checksum(
                                        line.line_number(),
                                        line.content(),
                                    ),
                                    id: Uuid::new_v4(),
                                }),
                            })
                            .expect("Cannot send message");
                    } else {
                        distributor
                            .send(EventInfo {
                                event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                                    state: BridgeState::ERRORED,
                                    description: StateDescription::Error {
                                        message: "Cannot resend line".to_string(),
                                    },
                                }),
                            })
                            .expect("Cannot send message");
                    }
                }
                return;
            }
        };
    }

    fn spawn_timeout(
        timeout_amount: u64,
        distributor: Sender<EventInfo>,
        state: Arc<Mutex<StateWrapper>>,
    ) {
        spawn(async move {
            sleep(Duration::from_secs(timeout_amount)).await;
            if state.lock().await.state == BridgeState::CONNECTING {
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
    }

    fn spawn_bridge_serial_reader(
        distributor: Sender<EventInfo>,
        bridge_sender: Sender<EventInfo>,
        print_info: Arc<Mutex<Option<PrintInfo>>>,
        state: Arc<Mutex<StateWrapper>>,
        canceled: Arc<Mutex<bool>>,
        queue: Arc<Mutex<VecDeque<Message>>>,
        ready: Arc<Mutex<bool>>,
        mut incoming: Box<dyn SerialPort>,
    ) {
        spawn(async move {
            let mut serial_buf: Vec<u8> = vec![0; 1];
            let mut collected = String::new();
            let mut has_collected_capabilities = false;
            let mut commands_left_to_send: Vec<String> = vec![];
            let cloned_dist = distributor.clone();
            let collected_responses: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(vec![]));
            loop {
                match incoming.read(serial_buf.as_mut_slice()) {
                    Ok(t) => {
                        if *canceled.lock().await {
                            break;
                        }
                        let data = String::from_utf8_lossy(&serial_buf[..t]);
                        let string = data.into_owned();
                        if string == "\n" {
                            if state.lock().await.state.eq(&BridgeState::CONNECTING) {
                                if collected.to_lowercase().starts_with("error") {
                                    distributor
                                        .send(EventInfo {
                                            event_type: EventType::Bridge(
                                                BridgeEvents::StateUpdate {
                                                    state: BridgeState::ERRORED,
                                                    description: StateDescription::Error {
                                                        message: collected.clone(),
                                                    },
                                                },
                                            ),
                                        })
                                        .expect("Cannot send message");
                                }
                                if has_collected_capabilities
                                    && state.lock().await.state.eq(&BridgeState::CONNECTING)
                                {
                                    collected = String::new();
                                    if commands_left_to_send.len() == 0 {
                                        distributor
                                            .send(EventInfo {
                                                event_type: EventType::Bridge(
                                                    BridgeEvents::StateUpdate {
                                                        state: BridgeState::CONNECTED,
                                                        description: api_manager::models::StateDescription::Capability {
                                                            capabilities: collected_responses.lock().await.clone()
                                                        },
                                                    },
                                                ),
                                            })
                                            .expect("Cannot send state update message");
                                        *collected_responses.lock().await = vec![];
                                        continue;
                                    }

                                    let command = commands_left_to_send.pop().unwrap();
                                    distributor
                                        .send(EventInfo {
                                            event_type: EventType::Bridge(
                                                BridgeEvents::TerminalSend {
                                                    message: command.clone(),
                                                    id: Uuid::new_v4(),
                                                },
                                            ),
                                        })
                                        .expect("Cannot send websocket message");

                                    continue;
                                }
                                // checking if capabilities are complete and parsing appropriate responses.
                                if collected.starts_with("ok") {
                                    if collected_responses.lock().await.len() == 0 {
                                        continue;
                                    }
                                    if !collected_responses.lock().await[0]
                                        .starts_with("FIRMWARE_NAME:Marlin")
                                    {
                                        *collected_responses.lock().await = vec![];
                                        incoming.write(b"M115\n").expect("Cannot resend M115.");
                                    } else {
                                        for cap in &*collected_responses.lock().await {
                                            // println!("[BRIDGE][CAP] => {}", cap);

                                            if cap.contains("Cap:AUTOREPORT_TEMP:1") {
                                                commands_left_to_send.push("M155 S2".to_string());
                                            } else {
                                                // TODO: ADD TEMP REPORTING.
                                                // let is_canceled = canceled.clone();
                                                // spawn(async move {
                                                //     loop {
                                                //         if *is_canceled.lock().await {
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
                                                        id: Uuid::new_v4(),
                                                    },
                                                ),
                                            })
                                            .expect("Cannot send message");

                                        collected_responses.lock().await.push(collected);
                                        has_collected_capabilities = true;
                                    }
                                } else {
                                    collected_responses.lock().await.push(collected);
                                }
                            } else {
                                if TOOLTEMPREGEX.is_match(&collected) {
                                    let temp_info = Parser::parse_temperature(&collected);
                                    cloned_dist
                                        .send(EventInfo {
                                            event_type: EventType::Websocket(temp_info),
                                        })
                                        .expect("Cannot send temp data");
                                } else {
                                    println!("[BRIDGE][RECV] {}", collected);
                                    collected_responses.lock().await.push(collected.clone());
                                    distributor
                                        .send(EventInfo {
                                            event_type: EventType::Websocket(
                                                WebsocketEvents::TerminalRead {
                                                    message: collected.clone(),
                                                },
                                            ),
                                        })
                                        .expect("Cannot send message");
                                }

                                if collected.starts_with("ok") {
                                    Bridge::handle_ok_response(
                                        &cloned_dist,
                                        &bridge_sender,
                                        &collected_responses,
                                        &mut collected,
                                        &state,
                                        &print_info,
                                        &queue,
                                        &ready,
                                    )
                                    .await;
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
                        std::io::ErrorKind::TimedOut => {
                            if *canceled.lock().await {
                                break;
                            } else {
                                yield_now().await;
                                if *ready.lock().await {
                                    let mut queue = queue.lock().await;
                                    let message = queue.pop_front();

                                    if message.is_some() {
                                        let message = message.unwrap();
                                        bridge_sender
                                            .send(EventInfo {
                                                event_type: EventType::Bridge(
                                                    BridgeEvents::TerminalSend {
                                                        message: message.content.clone(),
                                                        id: message.id,
                                                    },
                                                ),
                                            })
                                            .expect("Cannot send message");
                                    }
                                }
                            }
                        }
                        _ => {
                            if *canceled.lock().await {
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
    }

    fn spawn_event_listener(
        mut outgoing: Box<dyn SerialPort>,
        receiver: Receiver<EventInfo>,
        distributor: Sender<EventInfo>,
        print_info: Arc<Mutex<Option<PrintInfo>>>,
        state_info: Arc<Mutex<StateWrapper>>,
        canceled: Arc<Mutex<bool>>,
        queue: Arc<Mutex<VecDeque<Message>>>,
        ready: Arc<Mutex<bool>>,
    ) {
        spawn(async move {
            println!(
                "[BRIDGE] Connecting to port {} with {} baudrate",
                outgoing.as_ref().name().unwrap_or("UNNAMED".to_string()),
                outgoing.as_ref().baud_rate().unwrap()
            );
            loop {
                if let Ok(event) = receiver.try_recv() {
                    match event.event_type {
                        EventType::KILL => {
                            drop(outgoing);
                            *canceled.lock().await = true;
                            break;
                        }
                        EventType::Bridge(BridgeEvents::TerminalSend { mut message, id }) => {
                            if !message.ends_with("\n") {
                                message = format!("{}\n", message);
                            }
                            if state_info.lock().await.state.eq(&BridgeState::CONNECTED) {
                                if *ready.lock().await == false {
                                    let mut guard = queue.lock().await;
                                    guard.push_back(Message::new(message, id));
                                    continue;
                                } else {
                                    *ready.lock().await = true;
                                }
                            }
                            let result = outgoing.write(message.as_bytes());
                            if result.is_err() {
                                let err = result.unwrap_err();
                                eprintln!("[BRIDGE][ERROR] {}", err);
                                distributor
                                    .send(EventInfo {
                                        event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                                            state: BridgeState::ERRORED,
                                            description:
                                                api_manager::models::StateDescription::Error {
                                                    message: err.to_string(),
                                                },
                                        }),
                                    })
                                    .expect("Cannot send message");
                            } else {
                                println!("[BRIDGE][SEND] {}", message);
                                distributor
                                    .send(EventInfo {
                                        event_type: EventType::Websocket(
                                            WebsocketEvents::TerminalSend { message, id },
                                        ),
                                    })
                                    .expect("Cannot send message");
                            }
                        }
                        EventType::Bridge(BridgeEvents::PrintEnd) => {
                            if state_info.lock().await.state.ne(&BridgeState::PRINTING) {
                                return ();
                            }
                            *print_info.lock().await = None;
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
                            if state_info.lock().await.state.ne(&BridgeState::CONNECTED) {
                                return ();
                            }
                            let mut guard = print_info.lock().await;
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
                                        id: Uuid::new_v4(),
                                    }),
                                })
                                .expect("Cannot send first line");
                        }
                        EventType::Bridge(BridgeEvents::StateUpdate {
                            state,
                            description: _,
                        }) if state == BridgeState::CONNECTED => {
                            *ready.lock().await = true;
                        }
                        _ => (),
                    }
                } else {
                    yield_now().await;
                }
            }
        });
    }
}
