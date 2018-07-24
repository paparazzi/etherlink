extern crate ivyrust;
extern crate tun_tap;

use ivyrust::IvyMessage;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tun_tap::{Iface, Mode};

// TODO: make configurable
const AC_ID: u8 = 9;

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
        cb1.ivy_bind_msg(
            IvyMessage::callback,
            String::from("^") + &AC_ID.to_string() + " PAYLOAD (.*)",
        );
        loop {
            {
                let mut lock = cb1.data.lock();
                if let Ok(ref mut data) = lock {
                    if !data.is_empty() {
                        let payload_raw = &data.pop().unwrap()[0];
                        let payload_raw: Vec<&str> = payload_raw.split(",").collect();
                        let mut payload = vec![];
                        for byte in payload_raw {
                            match byte.parse::<u8>() {
                                Ok(val) => payload.push(val),
                                Err(e) => println!("Error parsing payload: {}", e),
                            }
                        }
                        let len = iface_writer.send(&payload).unwrap();
                        assert!(len == payload.len());
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
            let _size = iface_reader.recv(&mut buffer).unwrap();
            // now create a message
            let mut msg = "ground_dl PAYLOAD_COMMAND ".to_string() + &AC_ID.to_string() + " ";
            for byte in &buffer {
                msg = msg + &byte.to_string() + ",";
            }
            ivyrust::ivy_send_msg(msg);
            thread::sleep(Duration::from_millis(MSG_DELAY_MS));
        }
    });

    writer.join().unwrap();
    reader.join().unwrap();
}
