use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::engine::renderer_model::Image;

/// Stub
const IMAGE_DECODE_WORKERS: usize = 2;

struct DecodeCommand {
    source: String,
    bytes: Vec<u8>,
}

struct DecodeResult {
    source: String,
    result: anyhow::Result<Image>,
}

pub struct ImageDecoder {
    cmd_tx: Sender<DecodeCommand>,
    result_rx: Receiver<DecodeResult>,
}

impl Default for ImageDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageDecoder {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<DecodeCommand>();
        let (result_tx, result_rx) = mpsc::channel::<DecodeResult>();
        let cmd_rx = Arc::new(Mutex::new(cmd_rx));

        for _ in 0..IMAGE_DECODE_WORKERS {
            let cmd_rx = Arc::clone(&cmd_rx);
            let result_tx = result_tx.clone();
            thread::spawn(move || {
                loop {
                    let cmd = match cmd_rx.lock().unwrap().recv() {
                        Ok(cmd) => cmd,
                        Err(_) => break,
                    };
                    let result = Image::decode(&cmd.bytes);
                    let _ = result_tx.send(DecodeResult {
                        source: cmd.source,
                        result,
                    });
                }
            });
        }

        Self { cmd_tx, result_rx }
    }

    pub fn decode(&self, source: String, bytes: Vec<u8>) {
        let _ = self.cmd_tx.send(DecodeCommand { source, bytes });
    }

    pub fn try_receive(&self) -> Option<(String, anyhow::Result<Image>)> {
        self.result_rx.try_recv().ok().map(|r| (r.source, r.result))
    }
}
