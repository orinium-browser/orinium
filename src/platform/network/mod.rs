pub mod cache;
pub mod config;
pub mod cookie_store;
mod core;
pub mod error;
pub mod sender_pool;

// 外部公開用
pub use cache::Cache;
pub use config::NetworkConfig;
pub use cookie_store::CookieStore;
pub use core::Response;
pub use error::NetworkError;
pub use hyper::http::{Request, StatusCode};
pub use sender_pool::HostKey;
pub use sender_pool::{HttpSender, SenderPool};

use core::AsyncNetworkCore;

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

enum NetworkCommand {
    Fetch {
        url: String,
        reply: Sender<Result<Response, NetworkError>>,
    },
    SetConfig(NetworkConfig),
}

pub struct NetworkCore {
    tx: mpsc::Sender<NetworkCommand>,
}

impl NetworkCore {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        spawn_network_thread(rx);

        Self { tx }
    }

    pub fn set_network_config(&self, config: NetworkConfig) {
        let _ = self.tx.send(NetworkCommand::SetConfig(config));
    }

    pub fn fetch(&self, url: &str) -> Result<Response, NetworkError> {
        let (res_tx, res_rx) = mpsc::channel();

        self.tx
            .send(NetworkCommand::Fetch {
                url: url.to_string(),
                reply: res_tx,
            })
            .map_err(|_| NetworkError::Disconnected)?;

        res_rx.recv().map_err(|_| NetworkError::Disconnected)?
    }
}

fn spawn_network_thread(rx: Receiver<NetworkCommand>) {
    thread::spawn(move || {
        let mut core = AsyncNetworkCore::new();

        while let Ok(cmd) = rx.recv() {
            match cmd {
                NetworkCommand::Fetch { url, reply } => {
                    let res = core.fetch_blocking(&url);
                    let _ = reply.send(res);
                }
                NetworkCommand::SetConfig(cfg) => {
                    core.set_network_config(cfg);
                }
            }
        }
    });
}
