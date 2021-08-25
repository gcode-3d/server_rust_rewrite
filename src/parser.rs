use lazy_static::lazy_static;
use regex::Regex;
use serde::ser::SerializeStruct;

use crate::api_manager::models::{BridgeAction, WebsocketEvents};

lazy_static! {
    static ref TOOLTEMPREGEX: Regex = Regex::new(r"((T\d?):([\d\.]+) ?/([\d\.]+))+").unwrap();
    static ref BEDTEMPREGEX: Regex = Regex::new(r"B:([\d\.]+) ?/([\d\.]+)").unwrap();
    static ref CHAMBERREMPREGEX: Regex = Regex::new(r"((T\d?):([\d\.]+) ?/([\d\.]+))+").unwrap();
    static ref LINENR: Regex = Regex::new(r"ok N(\d+)").unwrap();
    static ref RESEND: Regex = Regex::new(r"Resend: N?:?(\d+)").unwrap();
}
pub struct Parser {}
impl Parser {
    pub fn parse_responses(responses: Vec<String>) -> BridgeAction {
        for response in responses {
            if RESEND.is_match(&response) {
                return BridgeAction::Resend(
                    RESEND.captures(&response).unwrap()[0]
                        .parse::<usize>()
                        .unwrap(),
                );
            }
            if response.starts_with("error") {
                if !response.starts_with("Error:Line Number is not Last Line Number+1, Last Line: ")
                {
                    return BridgeAction::Error;
                }
            }
        }
        return BridgeAction::Continue;
    }

    pub fn add_checksum(linenr: &usize, line: &str) -> String {
        let line = line.replace(" ", "");
        let line = format!("N{}{}", linenr, line);
        let mut cs: u8 = 0;
        for byte in line.bytes() {
            cs = cs ^ byte;
        }
        cs &= 0xff;
        return format!("{}*{}", line, cs);
    }

    pub fn parse_temperature(input: &String) -> WebsocketEvents {
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
        return WebsocketEvents::TempUpdate {
            tools,
            bed,
            chamber,
        };
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
