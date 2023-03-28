use std::ffi::{c_char, c_uchar, c_void, CStr, CString};
use std::net::UdpSocket;
use std::slice;
use std::thread::JoinHandle;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug)]
#[repr(C)]
pub enum ParserError {
    InvalidAddress,
    InvalidType,
    InvalidValue,
}

#[derive(Debug)]
#[repr(C)]
pub struct OscValue {
    int: i32,
    float: f32,
    bool: bool,
    string: *const c_char,
}

#[derive(Debug)]
#[repr(C)]
pub enum OscType {
    Int,
    Float,
    Bool,
    String,
}

#[repr(C)]
pub struct OscMessage {
    pub address: *const c_char,
    pub osc_type: OscType,
    pub value: OscValue,
    //raw: Vec<u8>,
}

fn extract_osc_address(buf: &[u8], ix: &mut usize) -> Result<String, ParserError> {
    // First, we wanna ensure the first char is a '/'
    if buf[0] != 47 {
        return Err(ParserError::InvalidAddress);
    }

    let mut address = String::new();

    while buf[*ix] != 0 {
        address.push(buf[*ix] as char);
        *ix += 1;
    }

    // Ensure we include the null terminator in the index
    *ix += 1;

    // Now round up to 4 bytes. If we're already on a 4 byte boundary, we don't need to do anything
    if *ix % 4 != 0 {
        *ix += 4 - (*ix % 4);
    }

    return Ok(address);
}

fn extract_osc_value(buf: &[u8], ix: &mut usize) -> Result<(OscType, OscValue), ParserError> {
    // First, we wanna ensure the first char is a ','
    if buf[*ix] != 44 {
        return Err(ParserError::InvalidType);
    }

    *ix += 1;

    let type_char = buf[*ix] as char;
    *ix += 3;

    let mut value = OscValue { int: 0, float: 0.0, bool: false, string: std::ptr::null() };

    // Now we convert this to an OscValue based on the type
    return match type_char {
        'i' => {
            let mut bytes = [0; 4];
            bytes.copy_from_slice(&buf[*ix..*ix + 4]);
            value.int = i32::from_be_bytes(bytes);
            Ok((OscType::Int, value))
        }
        'f' => {
            let mut bytes = [0; 4];
            bytes.copy_from_slice(&buf[*ix..*ix + 4]);
            value.float = f32::from_be_bytes(bytes);
            Ok((OscType::Float, value))
        }
        'T' => {
            value.bool = true;
            Ok((OscType::Bool, value))
        }
        'F' => {
            value.bool = false;
            Ok((OscType::Bool, value))
        }
        's' => {
            let mut string = String::new();
            while buf[*ix] != 0 {
                string.push(buf[*ix] as char);
                *ix += 1;
            }
            *ix += 1;
            value.string = CString::new(string).unwrap().into_raw();
            Ok((OscType::String, value))
        }
        _ => {
            Err(ParserError::InvalidType)
        }
    }
}

fn parse(buf: &[u8]) -> Result<OscMessage, ParserError> {
    let mut index = 0;
    let address = extract_osc_address(&buf, &mut index);
    println!("Address: {:?}", address);

    let value = extract_osc_value(&buf, &mut index);
    println!("Value: {:?}", value);

    return match (address, value) {
        (Ok(address), Ok(value)) => {
            Ok(OscMessage {
                address: CString::new(address).unwrap().into_raw(),
                osc_type: value.0,
                value: value.1,
                //raw: buf.to_vec(),
            })
        }
        (Err(e), _) => {
            Err(e)
        }
        (_, Err(e)) => {
            Err(e)
        }
    };
}

fn recv<F>(source: UdpSocket, mut callback: F)
    where
        F: FnMut(OscMessage),
{
    let mut buf: [u8; 4096] = [0; 4096];
    let (amt, _) = source.recv_from(&mut buf).unwrap();

    match parse(&buf[..amt]) {
        Ok(msg) => {
            callback(msg);
        }
        Err(e) => {
            println!("Error parsing message: {:?}", e);
        }
    }
}

#[no_mangle]
pub extern "C" fn start_socket(ip: *const c_char, port: u16, thread_ptr: *mut c_void, callback: extern "C" fn(*mut OscMessage)) -> i32 {
    let ip_address = match unsafe { CStr::from_ptr(ip) }.to_str() {
        Ok(ip) => ip,
        Err(_) => return -1, // Return error code -1 for invalid IP address
    };
    let address = format!("{}:{}", ip_address, port);
    let socket = match UdpSocket::bind(address) {
        Ok(socket) => socket,
        Err(_) => return -2, // Return error code -2 for socket binding error
    };
    // Start receiving thread
    let handle = std::thread::spawn(move || {
        recv(socket, |msg| {
            callback(Box::into_raw(Box::new(msg)));
        });
    });

    unsafe { *(thread_ptr as *mut *mut JoinHandle<()>) = Box::into_raw(Box::new(handle)) as *const c_void as *mut JoinHandle<()> };
    0
}

#[no_mangle]
pub extern "C" fn stop_socket(thread_ptr: *mut c_void) {
    // Get the thread handle from the provided pointer and join the thread
    let handle = unsafe { Box::from_raw(thread_ptr as *mut JoinHandle<()>) };
    handle.join().unwrap();
}

// Import a byte array from C# and parse it
#[no_mangle]
pub extern "C" fn parse_osc(buf: *const c_uchar, len: usize, msg: &mut OscMessage) -> bool {
    let buf = unsafe { slice::from_raw_parts(buf, len) };
    match parse(buf) {
        Ok(parsed_msg) => {
            *msg = parsed_msg; // update the provided OscMessage with the parsed message
            true
        }
        Err(_) => false,
    }
}

fn write_address(buf: &mut [u8], ix: &mut usize, address: &str) {
    let address_bytes = address.as_bytes();
    buf[*ix..*ix + address_bytes.len()].copy_from_slice(address_bytes);
    *ix += address_bytes.len();
    buf[*ix] = 0;
    *ix += 1;
    if *ix % 4 != 0 {
        *ix += 4 - (*ix % 4);
    }
}

#[no_mangle]
pub extern "C" fn create_osc_message(buf: *mut c_uchar, osc_template: &OscMessage) -> usize {
    let buf = unsafe { slice::from_raw_parts_mut(buf, 4096) };
    let address = unsafe { CStr::from_ptr(osc_template.address) }.to_str().unwrap();
    let mut ix = 0;
    write_address(buf, &mut ix, address);
    buf[ix] = 44; // ,
    ix += 1;
    match osc_template.osc_type {
        OscType::Int => {
            buf[ix] = 105; // i
            ix += 3;
            let bytes = osc_template.value.int.to_be_bytes();
            buf[ix..ix + 4].copy_from_slice(&bytes);
            ix += 4;
        }
        OscType::Float => {
            buf[ix] = 102; // f
            ix += 3;
            let bytes = osc_template.value.float.to_be_bytes();
            buf[ix..ix + 4].copy_from_slice(&bytes);
            ix += 4;
        }
        OscType::Bool => {
            buf[ix] = if osc_template.value.bool { 84 } else { 70 }; // T or F
            ix += 3;
        }
        OscType::String => {
            println!("Not implemented yet!")
        }
    }

    ix
}

// Creates a bundle from an array of OscMessages
#[no_mangle]
pub extern "C" fn create_osc_bundle(buf: *mut c_uchar, messages: *const OscMessage, len: usize, messages_index: *mut usize) -> usize {
    // OSC bundles start with the 16 byte header consisting of "#bundle" (with null terminator) followed by a 64-bit big-endian timetag
    let buf = unsafe { slice::from_raw_parts_mut(buf, 4096) };
    let messages = unsafe { slice::from_raw_parts(messages, len) };
    let mut ix = 0;

    // Write the header
    buf[0..8].copy_from_slice(b"#bundle\0");
    ix += 8;

    // Write the current NTP time as the timetag
    let time = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap();

    // Ensure we don't overflow the 64-bit integer
    let time = (time.as_secs() as u64) << 32 | (time.subsec_nanos() as u64) << 32 >> 32;

    let bytes = time.to_be_bytes();
    buf[ix..ix + 8].copy_from_slice(&bytes);
    ix += 8;

    // Now we need to write the messages
    let mut message_ix = unsafe { *messages_index };
    for msg in messages.iter().skip(message_ix) {
        // We need to calculate the length of the string and pad it to a multiple of 4 to ensure alignment
        // then add another 4 bytes for the length of the message
        // If adding it would go over the buffer size, return
        // Use the existing function to write the message to the buffer
        let address = unsafe { CStr::from_ptr(msg.address).to_str() }.unwrap();
        let length = address.len() + 1;
        let padded_length = if length % 4 == 0 { length } else { length + 4 - (length % 4) };
        if ix + padded_length + 4 > 4096 {
            return ix;
        }

        let length = create_osc_message(unsafe { buf.as_mut_ptr().add(ix + 4) }, msg);
        // Write the length of the message to the buffer. Ensure we use 4 bytes
        let bytes: [u8; 4] = (length as u32).to_be_bytes();

        buf[ix..ix + 4].copy_from_slice(&bytes);
        ix += length + 4;

        // Update the message index after each iteration
        message_ix += 1;
    }

    // Update the messages_index pointer with the new message index
    unsafe { *messages_index = message_ix; }

    ix
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serialize_osc_message() {
        let mut buf: [u8; 4096] = [0; 4096];
        let osc_message = OscMessage {
            address: CString::new("/test_message/meme").unwrap().into_raw(),
            osc_type: OscType::Int,
            value: OscValue { int: 42, float: 0.0, bool: false, string: std::ptr::null_mut() },
        };

        create_osc_message(buf.as_mut_ptr(), &osc_message);
        match parse(&buf) {
            Ok(message) => {
                // Convert the address string ptr to a literal string and compare
                let address = unsafe { CStr::from_ptr(message.address) }.to_str().unwrap();
                assert_eq!(address, "/test_message/meme", "Address was resolved incorrectly.");
                assert_eq!(message.value.int, 42, "Value was resolved incorrectly.");
            }
            Err(_) => assert!(false, "Failed to parse message."),
        }
    }

    #[test]
    fn serialize_osc_bundle() {
        // Create an array consisting of three messages
        let osc_message1 = OscMessage {
            address: CString::new("/test_message/meme").unwrap().into_raw(),
            osc_type: OscType::Int,
            value: OscValue { int: 42, float: 0.0, bool: false, string: std::ptr::null_mut() },
        };
        let osc_message2 = OscMessage {
            address: CString::new("/test_message/meme2").unwrap().into_raw(),
            osc_type: OscType::Float,
            value: OscValue { int: 0, float: 3.14, bool: false, string: std::ptr::null_mut() },
        };
        let osc_message3 = OscMessage {
            address: CString::new("/test_message/meme3").unwrap().into_raw(),
            osc_type: OscType::Bool,
            value: OscValue { int: 0, float: 0.0, bool: true, string: std::ptr::null_mut() },
        };
        let messages = [osc_message1, osc_message2, osc_message3];

        let mut buf: [u8; 4096] = [0; 4096];
        let mut index: usize = 0;
        let len1 = create_osc_bundle(buf.as_mut_ptr(), messages.as_ptr(), messages.len(), &mut index);

        index = 1;
        let len2 = create_osc_bundle(buf.as_mut_ptr(), messages.as_ptr(), messages.len(), &mut index);

        assert!(len2 < len1, "Length of bundle was not calculated correctly. Second bundle should be smaller than the first.");
    }

    #[test]
    fn parse_bool() {
        let buf = [47, 116, 101, 115, 116, 0, 0, 0, 44, 84, 0, 0];
        match parse(&buf) {
            Ok(message) => {
                // Convert the address string ptr to a literal string and compare
                let address = unsafe { CStr::from_ptr(message.address) }.to_str().unwrap();
                assert_eq!(address, "/test", "Address was resolved incorrectly.");
                assert_eq!(message.value.bool, true, "Value was resolved incorrectly.");
            }
            Err(e) => {
                panic!("Error: {:?}", e);
            }
        }
    }

    #[test]
    fn parse_int() {
        let buf = [47, 116, 101, 115, 116, 0, 0, 0, 44, 105, 0, 0, 0, 0, 0, 9];
        match parse(&buf) {
            Ok(message) => {
                // Convert the address string ptr to a literal string and compare
                let address = unsafe { CStr::from_ptr(message.address) }.to_str().unwrap();
                assert_eq!(address, "/test", "Address was resolved incorrectly.");
                assert_eq!(message.value.int, 9, "Value was resolved incorrectly.");
            }
            Err(e) => {
                panic!("Error: {:?}", e);
            }
        }
    }

    #[test]
    fn parse_float() {
        // Get 69.42 as a big endian array of bytes
        let bytes = 69.42_f32.to_be_bytes();
        let buf = [47, 116, 101, 115, 116, 0, 0, 0, 44, 102, 0, 0];

        // Concatenate the two arrays
        let mut recv_bytes = [0; 16];
        recv_bytes[..12].copy_from_slice(&buf);
        recv_bytes[12..].copy_from_slice(&bytes);

        match parse(&recv_bytes) {
            Ok(message) => {
                // Convert the address string ptr to a literal string and compare
                let address = unsafe { CStr::from_ptr(message.address) }.to_str().unwrap();
                assert_eq!(address, "/test", "Address was resolved incorrectly.");
                assert_eq!(message.value.float, 69.42, "Value was resolved incorrectly.");
            }
            Err(e) => {
                panic!("Error: {:?}", e);
            }
        }
    }

    #[test]
    fn parse_string() {
        let buf = [47, 116, 101, 115, 116, 0, 0, 0, 44, 115, 0, 0, 104, 101, 108, 108, 111, 0, 0, 0];
        match parse(&buf) {
            Ok(message) => {
                // Convert the address string ptr to a literal string and compare
                let address = unsafe { CStr::from_ptr(message.address) }.to_str().unwrap();
                assert_eq!(address, "/test", "Address was resolved incorrectly.");
                // Convert the string ptr to a literal string and compare
                let string = unsafe { CStr::from_ptr(message.value.string) }.to_str().unwrap();
                assert_eq!(string, "hello", "Value was resolved incorrectly.");
            }
            Err(e) => {
                panic!("Error: {:?}", e);
            }
        }
    }
}
