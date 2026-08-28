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
use ipc_channel::IpcError;
pub use sender_pool::HostKey;
pub use sender_pool::{HttpSender, SenderPool};

use serde::{Deserialize, Serialize};

use core::{AsyncNetworkCore, SharedNetState};

use crate::ParentChannels;
use engine::background_worker::BackgroundWorker;
use ipc_channel::ipc::{IpcOneShotServer, IpcReceiver, IpcSender};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::{env, io, process};

#[derive(Deserialize, Serialize)]
pub enum NetworkCommand {
    Fetch {
        request: NetworkRequest,
        msg_id: usize,
    },
    SetConfig(NetworkConfig),
    ClearCache,
}

/// Serializable HTTP request passed to the network process.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct NetworkRequest {
    pub url: String,
    pub method: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl NetworkRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: "GET".to_string(),
            headers: Vec::new(),
            body: Vec::new(),
        }
    }
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

    /// Clears all cached responses in the network process.
    pub fn clear_cache(&self) {
        let _ = self.cmd_tx.send(NetworkCommand::ClearCache);
    }

    /// 非同期送信のみ。結果は try_receive で取得
    pub fn fetch_async(&self, url: String, msg_id: usize) {
        self.fetch_request_async(NetworkRequest::get(url), msg_id);
    }

    pub fn fetch_request_async(&self, request: NetworkRequest, msg_id: usize) {
        let _ = self.cmd_tx.send(NetworkCommand::Fetch { request, msg_id });
    }

    /// UIスレッドから呼ぶ: 完了しているメッセージを取り込む
    pub fn try_receive(&self) -> Vec<NetworkMessage> {
        let mut msgs = Vec::new();
        while let Ok(msg) = self.msg_rx.try_recv() {
            log::info!(
                target: "network",
                "return message for msg_id={}",
                msg.msg_id
            );
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

/// Fetch pool size: network work is IO-bound and benefits from some
/// over-subscription, but each worker holds its own tokio runtime, so the
/// count is capped well above the CPU count without growing unbounded.
fn network_worker_count() -> usize {
    std::thread::available_parallelism()
        .map(|parallelism| parallelism.get())
        .unwrap_or(2)
        .clamp(1, 6)
}

static NEXT_FETCH_WORKER_ID: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static FETCH_WORKER_ID: Cell<Option<usize>> = const { Cell::new(None) };
}

/// `[N]` prefix identifying the fetch pool worker emitting a log line.
///
/// Empty on threads outside the pool (the IPC loop), so those lines stay
/// unprefixed.
fn fetch_worker_tag() -> String {
    FETCH_WORKER_ID.with(|slot| match slot.get() {
        Some(id) => format!("[{id}] "),
        None => String::new(),
    })
}

/// ネットワークプロセスエントリ
///
/// Fetches run on a [`BackgroundWorker`] pool so one slow request no longer
/// blocks every other tab or subresource. Config and cache commands stay on
/// this thread: they mutate the state shared by all workers.
pub fn network_main(rx: IpcReceiver<NetworkCommand>, tx: IpcSender<NetworkMessage>) -> ! {
    let shared = Arc::new(SharedNetState::new());

    let worker = BackgroundWorker::new_with_init(
        network_worker_count(),
        {
            let shared = Arc::clone(&shared);
            move || {
                let id = NEXT_FETCH_WORKER_ID.fetch_add(1, Ordering::SeqCst);
                FETCH_WORKER_ID.with(|slot| slot.set(Some(id)));
                log::info!(target: "network", "[{id}] fetch worker started");
                AsyncNetworkCore::new(Arc::clone(&shared))
            }
        },
        move |core, (request, msg_id): (NetworkRequest, usize)| {
            let started = std::time::Instant::now();
            let response = core.fetch_request_blocking(&request);
            match &response {
                Ok(res) => log::info!(
                    target: "network",
                    "{}fetch completed: msg_id={} url={} status={} body={}B took={:?}",
                    fetch_worker_tag(),
                    msg_id,
                    res.url,
                    res.status.as_u16(),
                    res.body.len(),
                    started.elapsed(),
                ),
                Err(err) => log::warn!(
                    target: "network",
                    "{}fetch failed: msg_id={} error={} took={:?}",
                    fetch_worker_tag(),
                    msg_id,
                    err,
                    started.elapsed(),
                ),
            }
            let _ = tx.send(NetworkMessage { msg_id, response });
        },
    );

    while let Ok(cmd) = rx.recv() {
        match cmd {
            NetworkCommand::SetConfig(cfg) => shared.set_network_config(cfg),
            NetworkCommand::ClearCache => {
                shared.clear_cache();
                log::info!(target: "network", "cache cleared");
            }
            NetworkCommand::Fetch { request, msg_id } => {
                log::info!(
                    target: "network",
                    "fetch dispatched: msg_id={} url={} method={}",
                    msg_id,
                    request.url,
                    request.method
                );
                worker.send((request, msg_id));
            }
        }
    }

    drop(worker);

    let err = rx.recv().err().unwrap();

    if matches!(err, IpcError::Disconnected) {
        log::info!(target: "network", "IPC channel closed, exiting normally.");
        std::process::exit(0)
    } else {
        panic!("IPC channel unexpectedly closed: {err}")
    }
}
