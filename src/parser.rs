use std::{collections::VecDeque, sync::Arc, time::Duration};

use crossbeam_channel::Sender;
use lazy_static::lazy_static;
use regex::Regex;
use serde::ser::SerializeStruct;
use tokio::{
    sync::{Mutex, MutexGuard},
    task::yield_now,
    time::sleep,
};
use uuid::Uuid;

use crate::{
    api_manager::models::{
        BridgeEvents, EventInfo, EventType, Message, PrintInfo, StateDescription, WebsocketEvents,
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
pub struct Parser {}
impl Parser {
    pub fn new() -> Self {
        Self {}
    }

    pub async fn parse_line(
        &mut self,
        distributor: &Sender<EventInfo>,
        sender: &Sender<EventInfo>,
        input: &String,
        state: BridgeState,
        print_info: Arc<Mutex<Option<PrintInfo>>>,
        ready_for_input: Arc<Mutex<bool>>,
        queue: Arc<Mutex<VecDeque<Message>>>,
    ) -> () {
        if input.trim().starts_with("ok") {
            let message = queue.lock().await.pop_front();

            if message.is_some() {
                let message = message.unwrap();
                *ready_for_input.lock().await = true;
                Parser::send_raw(&distributor, message.clone());
                return;
            } else {
                *ready_for_input.lock().await = true;
            }
        }
        if !TOOLTEMPREGEX.is_match(input) {
            println!("[BRIDGE][RECV] {}", input);
        }
        if TOOLTEMPREGEX.is_match(input) {
            Parser::handle_temperature_message(distributor, input);
        } else if state == BridgeState::PRINTING && RESEND.is_match(input) {
            let number = RESEND.captures(input).unwrap()[1].parse::<usize>().unwrap();
            println!("[PARSER][RESEND] Getting a resend for: {}", number);
            yield_now().await;
            sleep(Duration::from_secs(1)).await;
            let mut guard = print_info.lock().await;
            let print_info = guard.as_mut().unwrap();

            let resend_content = print_info.get_line_by_index(number);
            if resend_content.is_none() {
                distributor.send(EventInfo {
                    event_type: EventType::Bridge(BridgeEvents::StateUpdate {
                        state: BridgeState::ERRORED,
                        description: StateDescription::Error { message: "Requested resend message is not found in gcode cache\nStopping print".to_string() },
                    }),
                }).expect("Cannot send message");
                return;
            }
            let resend_content = resend_content.unwrap().clone();
            print_info.set_line_number(number);
            Parser::send_raw(
                &distributor,
                Message::new(resend_content.to_string(), Uuid::new_v4()),
            );
        } else if state == BridgeState::PRINTING && input.trim().starts_with("ok") {
            let guard = print_info.lock().await;
            if guard.is_some() {
                Parser::handle_print(distributor, sender, guard);
            }
            Parser::send_output_to_web(distributor, input);
        } else if state == BridgeState::PRINTING
            && input
                .trim()
                .to_lowercase()
                .starts_with("echo:busy: processing")
        {
            sleep(Duration::from_secs(1)).await;
        } else if input.to_lowercase().starts_with("error") {
            if input.starts_with("Error:Line Number is not Last Line Number+1,") {
                return;
            }
            println!("[RECV][ERR] {}", input);
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
            Parser::send_output_to_web(distributor, input);
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
        sender: &Sender<EventInfo>,
        mut print_info_guard: MutexGuard<Option<PrintInfo>>,
    ) -> () {
        loop {
            let line = print_info_guard.as_mut().unwrap().get_next_line();

            if line.is_none() {
                distributor
                    .send(EventInfo {
                        event_type: EventType::Bridge(BridgeEvents::PrintEnd),
                    })
                    .expect("Cannot send message");
                break;
            }
            let line = line.unwrap();
            let info = print_info_guard.as_mut().unwrap();
            let prev_progress = format!("{:.1}", info.progress());

            info.add_bytes_sent(line.content().len() as u64);
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

            let message = Parser::add_checksum(line.line_number(), line.content());

            sender
                .send(EventInfo {
                    event_type: EventType::Bridge(BridgeEvents::TerminalSend {
                        message,
                        id: Uuid::new_v4(),
                    }),
                })
                .expect("Cannot send file line");
            break;
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
        Parser::send_output_to_web(distributor, &message.content);
    }

    fn add_checksum(linenr: &usize, line: &str) -> String {
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
