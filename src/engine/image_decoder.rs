use crate::engine::background_worker::BackgroundWorker;
use crate::engine::renderer_model::Image;

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
    worker: BackgroundWorker<DecodeCommand, DecodeResult>,
}

impl Default for ImageDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl ImageDecoder {
    pub fn new() -> Self {
        Self {
            worker: BackgroundWorker::new(IMAGE_DECODE_WORKERS, |cmd: DecodeCommand| {
                DecodeResult {
                    source: cmd.source,
                    result: Image::decode(&cmd.bytes),
                }
            }),
        }
    }

    pub fn decode(&self, source: String, bytes: Vec<u8>) {
        self.worker.send(DecodeCommand { source, bytes });
    }

    pub fn try_receive(&self) -> Option<(String, anyhow::Result<Image>)> {
        self.worker.try_receive().map(|r| (r.source, r.result))
    }
}
