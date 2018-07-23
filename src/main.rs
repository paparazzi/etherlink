extern crate ivyrust;
extern crate tun_tap;

use ivyrust::IvyMessage;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tun_tap::{Iface, Mode};

const AC_ID: u8 = 9;
/// The packet data. Note that it is prefixed by 4 bytes ‒
/// two bytes are flags, another two are
/// protocol. 8, 0 is IPv4, 134, 221 is IPv6.
/// See <https://en.wikipedia.org/wiki/EtherType#Examples>.
const OFFSET: usize = 4;

/// 255 bytes - header (8 bytes) = 247 bytes
/// see https://wiki.paparazziuav.org/wiki/Messages_Format
const MAX_DATA_LEN: usize = 247;

fn main() {
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
        //cb1.ivy_bind_msg(IvyMessage::callback, String::from("^(\\S*) DL_VALUES (.*)"));
        cb1.ivy_bind_msg(IvyMessage::callback, String::from("^(\\S*) PAYLOAD (.*)"));
        loop {
            let mut lock = cb1.data.lock();
            if let Ok(ref mut data) = lock {
                if !data.is_empty() {
                    let payload_raw = &data.pop().unwrap()[1]; // this is now a string of comma separated values
                    println!("Original paylaod = {}", payload_raw);
                    let payload_raw: Vec<&str> = payload_raw.split(",").collect();
                    println!("Split paylaod = {:?}", payload_raw);
                    let mut payload = vec![];
                    for byte in payload_raw {
                        println!("Parsing {}", byte);
                        match byte.parse::<u8>() {
                            Ok(val) => payload.push(val),
                            Err(e) => println!("Error parsing payload: {}", e),
                        }
                    }
                    println!("Parsed payload = {:?}", payload);
                    println!("Payload len = {}", payload.len());
                    let len = iface_writer.send(&payload).unwrap();
                    assert!(len == payload.len());
                }
            }
            // TODO: introduce a delay to avoid sending too many messages?
            thread::sleep(Duration::from_millis(100));
        }
    });

    // Reads data from tap interface, coverts them to strings and then
    // creates a new PAYLOAD_COMMAND message and sends it over Ivy bus
    // so the link program can forward it to the autopilot
    let reader = thread::spawn(move || {
        let mut buffer = vec![0; MAX_DATA_LEN];
        loop {
            let size = iface_reader.recv(&mut buffer).unwrap();
            // TODO: warn if too many data are attempted to be send?
            println!("Rx bytes: {}", size - OFFSET);

            // now create a message
            let mut msg = "ground_dl PAYLOAD_COMMAND ".to_string() + &AC_ID.to_string() + " ";
            for byte in buffer[OFFSET..size].iter() {
                msg = msg + &byte.to_string() + ",";
            }
            //println!("Sending {}", msg);
            ivyrust::ivy_send_msg(msg);
            // TODO: introduce a delay to avoid sending too many messages?
            thread::sleep(Duration::from_millis(100));
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();
}
