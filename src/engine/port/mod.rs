//! Ports for moving work between threads, and later between processes.
//!
//! The browser currently runs everything in one process, so "where does this
//! code run" is answered by a thread id and nothing else. That answer stops
//! working the moment a component is split out: a call that used to be a
//! function call becomes a message, and the code that sends it has to stop
//! assuming the callee shares its memory.
//!
//! [`Outbox`] and [`Mailbox`] are the two ends of a one-way channel; the
//! [`Transport`] between them decides where the messages actually travel. All
//! three are plain values, so a caller keeps working whether the far end is
//! another thread in this process or a peer in another one.
//!
//! The scope is deliberately narrow. This is a *placement* port, not a
//! serialisation port: nothing here decides how a message is encoded, and a
//! cross-process transport will additionally need `Serialize`/`Deserialize` on
//! the message types. Some engine types cannot cross that boundary at all yet
//! — see [`SendableResult`](crate::engine::layouter::processor) and the `Rc` in
//! DOM trees.

pub mod mailbox;
pub mod transport;

pub use mailbox::{Consume, Disconnected, Mailbox, Outbox, Produce, Undeliverable};
pub use transport::{InProcess, Transport};
