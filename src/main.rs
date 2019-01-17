extern crate ivyrust;
extern crate tun_tap;

use ivyrust::IvyMessage;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tun_tap::{Iface, Mode};

#[macro_use]
extern crate clap;
use clap::App;

#[macro_use]
extern crate lazy_static;

/// Delay between processing messages, to avoid
/// congesting Pprzlink
const MSG_DELAY_MS: u64 = 100;

/// 255 bytes - header (8 bytes) = 247 bytes
/// see https://wiki.paparazziuav.org/wiki/Messages_Format
/// Minimal MTU is 68 bytes, see RFC 791 at
/// https://tools.ietf.org/html/rfc791
/// However, ssh/scp etc. doesn't work with such little (247) MTU
/// and requires MTU of 576 bytes. One way to handle this is to
/// internally fragment and reassemble packets.
const MAX_PAYLOAD_LEN: usize = 245; // 1 byte header, 246 bytes data

const MAX_DATA_LEN: usize = 600; // min length that SSH supports

const MORE_FLAGS: u8 = 0x80;

const NO_MORE_FLAGS: u8 = 0;

lazy_static! {
    static ref PACKET: Arc<Mutex<Packet>> = Arc::new(Mutex::new(Packet::new()));
}

struct Packet {
    last_seq: u8,
    buffer: Vec<u8>,
    more: bool,
}

impl Packet {
    fn new() -> Packet {
        Packet {
            last_seq: 0,
            buffer: vec![],
            more: false,
        }
    }
    fn reset(&mut self) {
        self.last_seq = 0;
        self.more = false;
        self.buffer = vec![];
    }
}

/// Ressemble smaller Pprzlink packets into a larger IP packet
fn assemble_algo(mut payload: Vec<u8>, _debug: bool) -> Option<Vec<u8>> {
    let mut packet = PACKET.lock().unwrap();

    let header = payload.remove(0);
    let seq = header & 0x7F;
    let more = header & 0x80;

    // is empty packet? insert the payload
    if packet.buffer.is_empty() && seq == 0 {
        // check for more flags
        if more == MORE_FLAGS {
            packet.buffer.append(&mut payload);
            packet.last_seq = seq;
            packet.more = true;
            return None;
        } else {
            // single packet, return payload
            return Some(payload);
        }
    }

    // we have an existing packet, check the seq number
    if seq == packet.last_seq + 1 {
        // correct seq number
        if more == NO_MORE_FLAGS {
            // this is the last fragment
            packet.buffer.append(&mut payload);
            let payload = packet.buffer.clone();
            packet.reset();
            return Some(payload);
        } else {
            // this is not the last packet
            packet.buffer.append(&mut payload);
            packet.last_seq = seq;
            packet.more = true;
            return None;
        }
    }

    // if we are here, something bad happen, so reset
    packet.reset();
    None
}

/// Cut larger (up to 576 bytes) IP packet into smaller Pprzlink packets
fn dissassemble_algo(buffer: &[u8], debug: bool, ac_id: u8) -> Vec<String> {
    let mut msgs = vec![];

    // receive a packet that is up to MAX_DATA_LEN long
    let size = buffer.len();
    if debug {
        println!("size ={}", size);
    }

    let mut start = 0; // start index in the `buffer`
    let mut end = size - 1; // end index in the `buffer`
    let mut more = true; // are there more fragments?
    let mut seq = 0; // sequence number
    let mut header; // header byte

    let mut cnt = 1;
    while more {
        if debug {
            println!("Round {}", cnt);
            println!("start={},end={},seq={},more={}", start, end, seq, more);
            println!("size-start={}", size - start);
        }

        if (size - start) >= MAX_PAYLOAD_LEN {
            // there will be more fragments
            if debug {
                println!("There will be more fragments");
            }
            header = MORE_FLAGS;
            end = start + MAX_PAYLOAD_LEN - 1; // leave space for the header
        } else {
            if debug {
                println!("No more fragments");
            }
            more = false;
            header = NO_MORE_FLAGS;
            // update end index, start index is untouched
            end = size;
        }
        if debug {
            println!("actual start={}, actual end={}", start, end);
        }
        
        // add seq number
        header += seq;
        // now create a message
        let mut msg = "ground_dl PAYLOAD_COMMAND ".to_string() + &ac_id.to_string() + " ";
        // insert header to the beginning of the array
        msg = msg + &header.to_string() + ",";
        // insert rest of the payload
        for byte in &buffer[start..end] {
            msg = msg + &byte.to_string() + ",";
        }
        // send ivy message
        msgs.push(msg);
        // increment seq
        seq += 1;
        // increment start
        start += buffer[start..end].len();
        if start > size {
            start = size;
        }
        cnt += 1;
    }
    msgs
}

fn main() {
    let yaml = load_yaml!("../cli.yml");
    let matches = App::from_yaml(yaml).get_matches();

    let ac_id: u8 = matches.value_of("AC_ID").unwrap().parse().unwrap();
    let debug = matches.is_present("debug");

    let _ = thread::spawn(move || {
        ivyrust::ivy_init(
            "payload_forwarder".to_string(),
            "forwarder ready".to_string(),
        );
        ivyrust::ivy_start(None);
        ivyrust::ivy_main_loop().unwrap();
    });

    let iface = Arc::new(Iface::new("tap1", Mode::Tap).unwrap());

    let iface_writer = Arc::clone(&iface);
    let iface_reader = Arc::clone(&iface);

    // Binds on PAYLOAD message, converts the payload into bytes and sends writes
    // it to the tap interface
    let writer = thread::spawn(move || {
        let mut cb1 = IvyMessage::new();
        cb1.ivy_bind_msg(
            IvyMessage::callback,
            String::from("^") + &ac_id.to_string() + " PAYLOAD (.*)",
        );
        loop {
            {
                let mut lock = cb1.data.lock();
                if let Ok(ref mut data) = lock {
                    if !data.is_empty() {
                        if debug {
                            println!("data={:?}", *data);
                        }
                        let payload_raw = &data.pop().unwrap()[0];
                        if debug {
                            println!("payload raw len = {}", payload_raw.len());
                            println!("payload raw = {:?}", payload_raw);
                        }
                        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
                        let mut payload = vec![];
                        for byte in payload_raw {
                            match byte.parse::<u8>() {
                                Ok(val) => payload.push(val),
                                Err(e) => println!("Error parsing payload: {}", e),
                            }
                        }
                        if let Some(msg) = assemble_algo(payload, debug) {
                            if debug {
                                println!("To iface len: {}", msg.len());
                                println!("To iface msg: {:?}",msg);
                            }
                            let len = iface_writer.send(&msg).unwrap();
                            assert!(len == msg.len());
                        }
                    }
                }
            }
            thread::sleep(Duration::from_millis(MSG_DELAY_MS));
        }
    });

    // Reads data from tap interface, coverts them to strings and then
    // creates a new PAYLOAD_COMMAND message and sends it over Ivy bus
    // so the link program can forward it to the autopilot
    let reader = thread::spawn(move || {
        let mut buffer = vec![0; MAX_DATA_LEN];
        loop {
            // receive a packet that is up to MAX_DATA_LEN long
            let size = iface_reader.recv(&mut buffer).unwrap();
            if debug {
                println!("From iface len: {}", size);
                println!("FRom iface msg: {:?}",&buffer[..size]);
            }
            
            let msgs = dissassemble_algo(&buffer[..size], debug, ac_id);
            for msg in msgs {
                if debug {
                    println!("ivy msg = {}",msg);
                }
                println!("ivy msg len = {}",msg.len());
                println!("ivy msg = {}",msg);

                // send ivy message
                ivyrust::ivy_send_msg(msg);
                // congestion control, sleep a bit
                thread::sleep(Duration::from_millis(MSG_DELAY_MS));
            }
            // optional: sleep here as well
            //thread::sleep(Duration::from_millis(MSG_DELAY_MS));
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();
}

#[cfg(test)]
mod test {
    use super::*;

    const AC_ID: u8 = 9;


    #[test]
    fn test_single_assembly() {
        let payload_raw = String::from("0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,");
        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
        let mut payload = vec![];
        for byte in payload_raw {
            match byte.parse::<u8>() {
                Ok(val) => payload.push(val),
                Err(e) => println!("Error parsing payload: {}", e),
            }
        }
        let msg = assemble_algo(payload, true).unwrap();
        println!("{:?}",msg);
        assert_eq!(msg.len(),97);
    }

    #[test]
    fn test_two_msg_assembly() {
        let payload_raw = String::from("128,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,240,241,242,243,244,245,246");
        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
        let mut payload = vec![];
        for byte in payload_raw {
            match byte.parse::<u8>() {
                Ok(val) => payload.push(val),
                Err(e) => println!("Error parsing payload: {}", e),
            }
        }
        let res = assemble_algo(payload, true);
        assert_eq!(None,res);

        let payload_raw = String::from("1,247,248,249,250,251,252,");
        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
        let mut payload = vec![];
        for byte in payload_raw {
            match byte.parse::<u8>() {
                Ok(val) => payload.push(val),
                Err(e) => println!("Error parsing payload: {}", e),
            }
        }
        let msg = assemble_algo(payload, true).unwrap();
        println!("{:?}",msg);
        assert_eq!(msg.len(),252);
    }

    #[test]
    fn test_three_msg_assembly() {
        let payload_raw = String::from("128,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239,240,241,242,243,244,245,246");
        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
        let mut payload = vec![];
        for byte in payload_raw {
            match byte.parse::<u8>() {
                Ok(val) => payload.push(val),
                Err(e) => println!("Error parsing payload: {}", e),
            }
        }
        let res = assemble_algo(payload, true);
        assert_eq!(None,res);

        let payload_raw = String::from("129,247,248,249,250,251,252,253,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,70,71,72,73,74,75,76,77,78,79,80,81,82,83,84,85,86,87,88,89,90,91,92,93,94,95,96,97,98,99,100,101,102,103,104,105,106,107,108,109,110,111,112,113,114,115,116,117,118,119,120,121,122,123,124,125,126,127,128,129,130,131,132,133,134,135,136,137,138,139,140,141,142,143,144,145,146,147,148,149,150,151,152,153,154,155,156,157,158,159,160,161,162,163,164,165,166,167,168,169,170,171,172,173,174,175,176,177,178,179,180,181,182,183,184,185,186,187,188,189,190,191,192,193,194,195,196,197,198,199,200,201,202,203,204,205,206,207,208,209,210,211,212,213,214,215,216,217,218,219,220,221,222,223,224,225,226,227,228,229,230,231,232,233,234,235,236,237,238,239");
        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
        let mut payload = vec![];
        for byte in payload_raw {
            match byte.parse::<u8>() {
                Ok(val) => payload.push(val),
                Err(e) => println!("Error parsing payload: {}", e),
            }
        }
        let res = assemble_algo(payload, true);
        assert_eq!(None,res);


        let payload_raw = String::from("2,240,241,242,243,244,245,246,247,248,249,250,251,252,253,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39,40,41,42,43,44,45,46,47,48,49,50,51,52,53,54,55,56,57,58,59,60,61,62,63,64,65,66,67,68,69,");
        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
        let mut payload = vec![];
        for byte in payload_raw {
            match byte.parse::<u8>() {
                Ok(val) => payload.push(val),
                Err(e) => println!("Error parsing payload: {}", e),
            }
        }
        let msg = assemble_algo(payload, true).unwrap();
        println!("{:?}",msg);
        assert_eq!(msg.len(),575);
    }

    

    #[test]
    fn test_single_seq() {
        let mut buf = vec![];
        for idx in 1..99 {
            buf.push(idx);
        }
        let res = dissassemble_algo(&buf, true, AC_ID);
        println!("{:?}", res);
        assert_eq!(res.len(), 1);
    }

    #[test]
    fn test_multiple_seq() {
        let mut buf = vec![];
        for idx in 1..254 {
            buf.push(idx);
        }
        let res = dissassemble_algo(&buf, true, AC_ID);
        println!("{:?}", res);
        assert_eq!(res.len(), 2);
    }

    #[test]
    fn test_full_length() {
        let mut buf = vec![];
        for idx in 1..255 {
            buf.push(idx);
        }
        for idx in 1..255 {
            buf.push(idx);
        }
        for idx in 1..255 {
            buf.push(idx);
        }
        let res = dissassemble_algo(&buf[..MAX_DATA_LEN], true, AC_ID);
        println!("{}", res[0]);
        println!("{}", res[1]);
        println!("{}", res[2]);
        assert_eq!(res.len(), 3);
    }


    #[test]
    fn test_ping() {
        let buf = [0, 0, 8, 6, 255, 255, 255, 255, 255, 255, 54, 181, 4, 231, 234, 212, 8, 6, 0, 1, 8, 0, 6, 4, 0, 1, 54, 181, 4, 231, 234, 212, 192, 168, 69, 1, 0, 0, 0, 0, 0, 0, 192, 168, 69, 2];
        let res = dissassemble_algo(&buf, true, AC_ID);
        println!("res={:?}",res);
        assert_eq!(res.len(), 1);
    }
}
