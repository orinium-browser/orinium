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

pub enum NetworkCommand {
    Fetch { url: String, msg_id: usize },
    SetConfig(NetworkConfig),
}

pub struct NetworkMessage {
    pub msg_id: usize,
    pub response: Result<Response, NetworkError>,
}

pub struct NetworkCore {
    cmd_tx: Sender<NetworkCommand>,
    msg_rx: Receiver<NetworkMessage>, // UI スレッド用
}

impl Default for NetworkCore {
    fn default() -> Self {
        Self::new()
    }
}

impl NetworkCore {
    pub fn new() -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (msg_tx, msg_rx) = mpsc::channel();

        thread::spawn(move || spawn_network_thread(cmd_rx, msg_tx));

        Self { cmd_tx, msg_rx }
    }

    pub fn set_network_config(&self, cfg: NetworkConfig) {
        let _ = self.cmd_tx.send(NetworkCommand::SetConfig(cfg));
    }

    /// 非同期送信のみ。結果は try_receive で取得
    pub fn fetch_async(&self, url: String, msg_id: usize) {
        let _ = self.cmd_tx.send(NetworkCommand::Fetch { url, msg_id });
    }

    /// UIスレッドから呼ぶ: 完了しているメッセージを取り込む
    pub fn try_receive(&self) -> Vec<NetworkMessage> {
        let mut msgs = Vec::new();
        while let Ok(msg) = self.msg_rx.try_recv() {
            println!("NetworkCore: received message for msg_id={}", msg.msg_id);
            msgs.push(msg);
        }
        msgs
    }

    pub fn fetch_blocking(&self, url: &str) -> Result<Response, NetworkError> {
        self.fetch_async(url.to_string(), 0);
        loop {
            if let Some(v) = self.try_receive().into_iter().next() {
                return v.response;
            }
            std::thread::yield_now();
        }
    }
}

/// ネットワークスレッド
fn spawn_network_thread(rx: Receiver<NetworkCommand>, tx: Sender<NetworkMessage>) {
    let mut core = AsyncNetworkCore::new();

    for cmd in rx {
        match cmd {
            NetworkCommand::SetConfig(cfg) => core.set_network_config(cfg),
            NetworkCommand::Fetch { url, msg_id } => {
                let res = core.fetch_blocking(&url);
                println!("NetworkCore: fetched URL for msg_id={}", msg_id);
                let _ = tx.send(NetworkMessage {
                    msg_id,
                    response: res,
                });
            }
        }
    }
}
