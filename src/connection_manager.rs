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
