use serialport::SerialPortInfo;

enum ConnectionStates {
    DISCONNECTED = 0,
    CONNECTED = 1,
    CONNECTING = 2,
    ERRORED = -1,
    PREPARING = 5,
    PRINTING = 6,
    FINISHING = 7,
}

pub struct ConnectionManager {
    state: ConnectionStates,
    port: Option<SerialPortInfo>,
}
impl ConnectionManager {
    pub fn new() -> Self {
        return Self {
            state: ConnectionStates::DISCONNECTED,
            port: None,
        };
    }
}

pub fn main() {
    //
    let ports = get_available_ports();

    for port in ports {
        print!("Port: {}", port.port_name);
    }
}

fn get_available_ports() -> Vec<serialport::SerialPortInfo> {
    return match serialport::available_ports() {
        Ok(ports) => ports,
        Err(_) => {
            let empty: Vec<serialport::SerialPortInfo> = vec![];
            return empty;
        }
    };
}
