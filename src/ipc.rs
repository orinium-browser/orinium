//! IPC module for inter-process communication via Unix domain sockets.
//!
//! The same binary is launched as either the "browser" process or the "network"
//! process.  Communication uses length-prefixed bincode frames over a local socket.
//!
//! Wire format: [4-byte message_len][bincode-encoded Message]

use std::io::{self, Read, Write};

#[derive(bincode::Encode, bincode::Decode)]
pub enum Message {
    FetchReq {
        id: u64,
        url: String,
    },
    FetchResp {
        id: u64,
        url: String,
        status: u16,
        reason_phrase: String,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    },
    ErrorResp {
        id: u64,
        error: String,
    },
}

pub fn send_msg(stream: &mut impl Write, msg: &Message) -> io::Result<()> {
    let encoded = bincode::encode_to_vec(msg, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    stream.write_all(&(encoded.len() as u32).to_le_bytes())?;
    stream.write_all(&encoded)?;
    Ok(())
}

pub fn recv_msg(stream: &mut impl Read) -> io::Result<Message> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let len = u32::from_le_bytes(len_buf) as usize;
    let mut buf = vec![0u8; len];
    stream.read_exact(&mut buf)?;
    let (msg, _) = bincode::decode_from_slice(&buf, bincode::config::standard())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok(msg)
}
