use crate::platform::network::{NetworkCommand, NetworkMessage};
use ipc_channel::ipc::{self, IpcSender};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct ChildChannels<R, S> {
    pub cmd_rx: ipc::IpcReceiver<R>,
    pub msg_tx: ipc::IpcSender<S>,
}

#[derive(Serialize, Deserialize)]
pub struct ParentChannels<R, S> {
    pub cmd_tx: ipc::IpcSender<R>,
    pub msg_rx: ipc::IpcReceiver<S>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessType {
    Network,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessHandler {
    kind: ProcessType,
    name: String,
}

impl ProcessHandler {
    pub fn current() -> Option<Self> {
        let kind = std::env::args().find_map(|arg| match arg.strip_prefix("--type=")? {
            "network" => Some(ProcessType::Network),
            _ => None,
        })?;

        let mut name = None;
        let mut args = std::env::args();
        while let Some(arg) = args.next() {
            if arg == "--child" {
                name = Some(args.next()?);
            }
        }
        let name = name?;

        Some(Self { kind, name })
    }

    pub fn handle(self) -> ! {
        match self.kind {
            ProcessType::Network => {
                network_process_main(self.name);
            }
        }
    }
}

fn network_process_main(name: String) -> ! {
    log::info!(target: "network", "network process started (pid={})", std::process::id());
    let (cmd_tx, cmd_rx) = ipc::channel::<NetworkCommand>()
        .inspect_err(|err| {
            log::error!(target: "NetworkProcess", "Failed to create IPC channel: {err}");
        })
        .expect("failed to create network IPC channel");
    let (msg_tx, msg_rx) = ipc::channel::<NetworkMessage>()
        .inspect_err(|err| {
            log::error!(target: "NetworkProcess", "Failed to create IPC channel: {err}");
        })
        .expect("failed to create network IPC channel");

    let tx: IpcSender<ParentChannels<NetworkCommand, NetworkMessage>> = IpcSender::connect(name)
        .inspect_err(|err| {
            log::error!(
                target: "NetworkProcess",
                "Failed to connect to parent IPC channel: {err}"
            );
        })
        .expect("failed to connect to parent IPC channel");
    tx.send(ParentChannels { cmd_tx, msg_rx })
        .inspect_err(|err| {
            log::error!(
                target: "NetworkProcess",
                "Failed to send IPC channels to parent process: {err}"
            );
        })
        .expect("failed to send IPC channels to parent process");

    crate::platform::network::network_main(cmd_rx, msg_tx)
}
