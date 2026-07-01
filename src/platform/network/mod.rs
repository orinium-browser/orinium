//! Network processing module, providing HTTP communication, cache, and cookie management.

pub mod cache;
pub mod config;
pub mod cookie_store;
mod core;
pub mod error;
pub mod sender_pool;

pub use cache::Cache;
pub use config::NetworkConfig;
pub use cookie_store::CookieStore;
pub use core::{Response, StatusCode};
pub use error::NetworkError;
pub use hyper::http::Request;
pub use sender_pool::HostKey;
pub use sender_pool::{HttpSender, SenderPool};

use serde::{Deserialize, Serialize};

use core::AsyncNetworkCore;

use crate::ParentChannels;
use ipc_channel::ipc::{IpcOneShotServer, IpcReceiver, IpcSender};
use std::{env, io, process};

#[derive(Deserialize, Serialize)]
pub enum NetworkCommand {
    Fetch { url: String, msg_id: usize },
    SetConfig(NetworkConfig),
}

#[derive(Deserialize, Serialize)]
pub struct NetworkMessage {
    pub msg_id: usize,
    pub response: Result<Response, NetworkError>,
}

pub struct NetworkCore {
    cmd_tx: IpcSender<NetworkCommand>,
    msg_rx: IpcReceiver<NetworkMessage>, // UI スレッド用
}

impl Default for NetworkCore {
    fn default() -> Self {
        Self::new().unwrap()
    }
}

impl NetworkCore {
    pub fn new() -> Result<Self, io::Error> {
        let (server, server_name) =
            IpcOneShotServer::<ParentChannels<NetworkCommand, NetworkMessage>>::new()?;

        process::Command::new(env::current_exe()?)
            .arg("--child")
            .arg(&server_name)
            .arg("--type=network")
            .spawn()?;

        let (_, channels) = server.accept().unwrap();

        Ok(Self {
            cmd_tx: channels.cmd_tx,
            msg_rx: channels.msg_rx,
        })
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
            log::info!("NetworkCore: received message for msg_id={}", msg.msg_id);
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
pub fn network_main(rx: IpcReceiver<NetworkCommand>, tx: IpcSender<NetworkMessage>) -> ! {
    let mut core = AsyncNetworkCore::new();

    while let Ok(cmd) = rx.recv() {
        match cmd {
            NetworkCommand::SetConfig(cfg) => core.set_network_config(cfg),
            NetworkCommand::Fetch { url, msg_id } => {
                let res = core.fetch_blocking(&url);
                log::info!("NetworkCore: fetched URL for msg_id={}", msg_id);
                let _ = tx.send(NetworkMessage {
                    msg_id,
                    response: res,
                });
            }
        }
    }
    panic!("IPC channel closed: {}", rx.recv().err().unwrap())
}
