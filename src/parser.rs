use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
    thread::sleep,
    time::Duration,
};

use crossbeam_channel::Sender;
use lazy_static::lazy_static;
use regex::Regex;
use serde::ser::SerializeStruct;
use uuid::Uuid;

use crate::{
    api_manager::models::{
        BridgeEvents, EventInfo, EventType, Message, PrintInfo, WebsocketEvents,
    },
    bridge::BridgeState,
};

lazy_static! {
    static ref TOOLTEMPREGEX: Regex = Regex::new(r"((T\d?):([\d\.]+) ?/([\d\.]+))+").unwrap();
    static ref BEDTEMPREGEX: Regex = Regex::new(r"B:([\d\.]+) ?/([\d\.]+)").unwrap();
    static ref CHAMBERREMPREGEX: Regex = Regex::new(r"((T\d?):([\d\.]+) ?/([\d\.]+))+").unwrap();
    static ref LINENR: Regex = Regex::new(r"ok N(\d+)").unwrap();
    static ref RESEND: Regex = Regex::new(r"Resend: N?:?(\d+)").unwrap();
}

pub fn parse_line(
    distributor: &Sender<EventInfo>,
    input: &String,
    state: BridgeState,
    print_info: Arc<Mutex<Option<PrintInfo>>>,
    ready_for_input: Arc<Mutex<bool>>,
    queue: Arc<Mutex<VecDeque<Message>>>,
) -> () {
    if input.trim().starts_with("ok") {
        let message = queue.lock().unwrap().pop_front();

        if message.is_some() {
            let message = message.unwrap();
            *ready_for_input.lock().unwrap() = true;
            send_raw(&distributor, message.clone());
            return;
        } else {
            *ready_for_input.lock().unwrap() = true;
        }
    }
    if TOOLTEMPREGEX.is_match(input) {
        handle_temperature_message(distributor, input);
    } else if state == BridgeState::PRINTING && input.trim().starts_with("ok") {
        let mut guard = print_info.lock().expect("Cannot lock print info");
        if guard.is_some() {
            if LINENR.is_match(input) {
                let number = LINENR.captures(input).unwrap()[1].parse::<u64>();
                if number.is_ok() {
                    guard.as_mut().unwrap().remove_sent_line(number.unwrap());
                }
            }

            handle_print(distributor, guard);
        }
        send_output_to_web(distributor, input);
    } else if state == BridgeState::PRINTING && RESEND.is_match(input) {
        let mut guard = print_info.lock().expect("Cannot lock print info");
        let number = RESEND.captures(input).unwrap()[1].parse::<u64>().unwrap();
        if guard.is_some() {
            let line = guard.as_mut().unwrap().get_sent_line(number);
            if line.is_some() {
                distributor
                    .send(EventInfo {
                        event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                            message: line.unwrap().to_string(),
                            id: Uuid::new_v4(),
                        }),
                    })
                    .expect("Cannot send file line");
            }
        }
    } else if state == BridgeState::PRINTING
        && input
            .trim()
            .to_lowercase()
            .starts_with("echo:busy: processing")
    {
        sleep(Duration::from_secs(1));
    } else if input.to_lowercase().starts_with("error") {
        distributor
            .send(EventInfo {
                event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                    state: BridgeState::ERRORED,
                    description: crate::api_manager::models::StateDescription::Error {
                        message: input.to_string(),
                    },
                }),
            })
            .expect("Cannot update state");
    } else {
        send_output_to_web(distributor, input);
    }
}

fn send_output_to_web(distributor: &Sender<EventInfo>, input: &String) {
    distributor
        .send(EventInfo {
            event_type: EventType::Websocket(WebsocketEvents::TerminalRead {
                message: input.clone(),
            }),
        })
        .expect("Cannot send message");
}

pub fn handle_print(
    distributor: &Sender<EventInfo>,
    mut print_info_guard: MutexGuard<Option<PrintInfo>>,
) -> () {
    loop {
        let line = print_info_guard
            .as_mut()
            .unwrap()
            .file_reader
            .as_mut()
            .expect("No file buffer loaded")
            .next();
        if line.is_none() {
            distributor
                .send(EventInfo {
                    event_type: EventType::Bridge(BridgeEvents::PrintEnd),
                })
                .expect("Cannot send message");
            break;
        }
        let line = line.unwrap();
        match line {
            Ok(line) => {
                let info = print_info_guard.as_mut().unwrap();
                let prev_progress = format!("{:.1}", info.progress());

                info.add_bytes_sent(line.len() as u64);
                let difference = format!("{:.1}", info.progress()).parse::<f64>().unwrap()
                    - prev_progress.parse::<f64>().unwrap();

                if difference > 0.1 {
                    distributor
                        .send(EventInfo {
                            event_type: EventType::Websocket(WebsocketEvents::StateUpdate {
                                state: BridgeState::PRINTING,
                                description: crate::api_manager::models::StateDescription::Print {
                                    filename: info.filename.to_string(),
                                    progress: info.progress(),
                                    start: info.start,
                                    end: info.end,
                                },
                            }),
                        })
                        .expect("Cannot send progress update");
                }
                if line.trim().starts_with(";") || line.trim().len().eq(&0) {
                    // Skipping comments & empty lines.
                    continue;
                }
                let linenr = print_info_guard.as_mut().unwrap().advance();
                let message = add_checksum(
                    linenr,
                    line.split_terminator(";").next().unwrap().to_string(),
                );
                print_info_guard
                    .as_mut()
                    .unwrap()
                    .insert_sent_line(linenr, message.clone());

                distributor
                    .send(EventInfo {
                        event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                            message,
                            id: Uuid::new_v4(),
                        }),
                    })
                    .expect("Cannot send file line");
                break;
            }
            Err(e) => eprintln!("[ERR][FILEPARSER] {}", e),
        }
    }
}

fn send_raw(distributor: &Sender<EventInfo>, message: Message) {
    distributor
        .send(EventInfo {
            event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                message: message.content.clone(),
                id: message.id,
            }),
        })
        .expect("Cannot send message");
    send_output_to_web(distributor, &message.content);
}

fn add_checksum(linenr: u64, line: String) -> String {
    let line = line.replace(" ", "");
    let line = format!("N{}{}", linenr, line);
    let mut cs: u8 = 0;
    for byte in line.bytes() {
        cs = cs ^ byte;
    }
    cs &= 0xff;
    return format!("{}*{}", line, cs);
}

fn handle_temperature_message(distributor: &Sender<EventInfo>, input: &String) -> () {
    let mut tools: Vec<TempInfo> = Vec::new();

    let mut bed: Option<TempInfo> = None;
    let mut chamber: Option<TempInfo> = None;

    for capture in TOOLTEMPREGEX.captures_iter(input) {
        let current_temp = capture[3].parse().unwrap_or(0.0);
        let target_temp = capture[4].parse().unwrap_or(0.0);

        let info = TempInfo::new(capture[2].to_string(), current_temp, target_temp);

        tools.push(info);
    }
    let bed_result = BEDTEMPREGEX.captures(input);
    if bed_result.is_some() {
        let bed_capture = bed_result.unwrap();
        let current_temp = bed_capture[1].parse().unwrap_or(0.0);
        let target_temp = bed_capture[2].parse().unwrap_or(0.0);

        let info = TempInfo::new_no_name(current_temp, target_temp);
        bed = Some(info);
    }

    let chamber_result = CHAMBERREMPREGEX.captures(input);
    if chamber_result.is_some() {
        let chamber_capture = chamber_result.unwrap();
        let current_temp = chamber_capture[1].parse().unwrap_or(0.0);
        let target_temp = chamber_capture[2].parse().unwrap_or(0.0);
        let info = TempInfo::new_no_name(current_temp, target_temp);
        chamber = Some(info);
    }

    distributor
        .send(EventInfo {
            event_type: EventType::Websocket(WebsocketEvents::TempUpdate {
                tools,
                bed,
                chamber,
            }),
        })
        .expect("Cannot send temperature message");
}

#[derive(Debug, Clone)]
pub struct TempInfo {
    tool_name: String,
    current_temp: f64,
    target_temp: f64,
}

impl serde::Serialize for TempInfo {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        if self.current_temp == 0.0 {
            return serializer.serialize_none();
        }
        if self.tool_name.len() == 0 {
            let mut state = serializer.serialize_struct("TempInfo", 2)?;
            state.serialize_field("currentTemp", &self.current_temp)?;
            state.serialize_field("targetTemp", &self.target_temp)?;
            state.end()
        } else {
            let mut state = serializer.serialize_struct("TempInfo", 2)?;
            state.serialize_field("name", &self.tool_name)?;
            state.serialize_field("currentTemp", &self.current_temp)?;
            state.serialize_field("targetTemp", &self.target_temp)?;
            state.end()
        }
    }
}

impl TempInfo {
    pub fn new(tool_name: String, current_temp: f64, target_temp: f64) -> Self {
        Self {
            tool_name,
            current_temp,
            target_temp,
        }
    }
    pub fn new_no_name(current_temp: f64, target_temp: f64) -> Self {
        Self {
            tool_name: String::new(),
            current_temp,
            target_temp,
        }
    }
}
