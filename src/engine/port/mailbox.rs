//! The two ends of a one-way message channel.
//!
//! [`Outbox`] is where messages are posted, [`Mailbox`] is where they are
//! collected. Both hide how the bytes actually travel, so a caller written
//! against them keeps working when the channel moves from this address space
//! to another process — see [`crate::engine::port::transport`].

use std::sync::Arc;

/// A message that never reached the far end of a channel.
///
/// The message is handed back rather than dropped: the producer may be the only
/// holder of it, and silently discarding work is worse than reporting that the
/// consumer is gone.
pub struct Undeliverable<T>(pub T);

// Spelled out rather than derived: the payload is deliberately not required to
// be `Debug`, since reporting the failure must not depend on the message type.
impl<T> std::fmt::Debug for Undeliverable<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Undeliverable(..)")
    }
}

impl<T: PartialEq> PartialEq for Undeliverable<T> {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl<T: Eq> Eq for Undeliverable<T> {}

impl<T> std::fmt::Display for Undeliverable<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the receiving end of the channel is gone")
    }
}

impl<T> std::error::Error for Undeliverable<T> {}

/// The far end of a channel disappeared before it could be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disconnected;

impl std::fmt::Display for Disconnected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the sending end of the channel is gone")
    }
}

impl std::error::Error for Disconnected {}

/// Posts messages into a channel.
///
/// Cheap to share: several producers may hold a clone, and `send` takes `&self`
/// so no producer needs unique access to post work.
pub struct Outbox<T> {
    inner: Arc<dyn Produce<T> + Send + Sync>,
}

impl<T: Send + 'static> Outbox<T> {
    /// Wraps a transport-specific producer.
    ///
    /// Transports build ends through this; everyone else gets them from
    /// [`Transport::channel`](crate::engine::port::transport::Transport::channel).
    pub fn of<P: Produce<T> + Send + Sync + 'static>(producer: P) -> Self {
        Self {
            inner: Arc::new(producer),
        }
    }

    /// Posts `message`, or hands it back if no consumer remains.
    ///
    /// Closing is reported by the send failing, so a producer learns about it
    /// without a separate query.
    pub fn send(&self, message: T) -> Result<(), Undeliverable<T>> {
        self.inner.send(message)
    }
}

impl<T> std::fmt::Debug for Outbox<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Outbox")
    }
}

impl<T> Clone for Outbox<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

/// Collects messages from a channel.
///
/// `recv` and `try_recv` both take `&self`, so a mailbox can be held behind a
/// shared reference and moved between threads. One consumer at a time is still
/// the rule: a transport whose consumer parks in `recv` may hold an internal
/// lock for the duration.
pub struct Mailbox<T> {
    inner: Arc<dyn Consume<T> + Send + Sync>,
}

impl<T: Send + 'static> Mailbox<T> {
    /// Wraps a transport-specific consumer.
    ///
    /// Transports build ends through this; everyone else gets them from
    /// [`Transport::channel`](crate::engine::port::transport::Transport::channel).
    pub fn of<C: Consume<T> + Send + Sync + 'static>(consumer: C) -> Self {
        Self {
            inner: Arc::new(consumer),
        }
    }

    /// Waits for the next message, parking rather than spinning.
    ///
    /// `Err(Disconnected)` means every producer is gone and the queue is
    /// drained — the only unambiguous way for a consumer to learn the channel
    /// is closed.
    pub fn recv(&self) -> Result<T, Disconnected> {
        self.inner.recv()
    }

    /// Takes the next message if one is already waiting, without blocking.
    ///
    /// The UI thread polls here rather than calling [`Mailbox::recv`], because
    /// it has to keep servicing the event loop between messages.
    pub fn try_recv(&self) -> Option<T> {
        self.inner.try_recv()
    }
}

impl<T> std::fmt::Debug for Mailbox<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mailbox")
    }
}

impl<T> Clone for Mailbox<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

/// A transport's producing half.
pub trait Produce<T>: Send + Sync {
    fn send(&self, message: T) -> Result<(), Undeliverable<T>>;
}

/// A transport's consuming half.
///
/// There is no `is_disconnected` here on purpose: a poll that finds an empty
/// queue cannot tell "nothing yet" from "nothing ever", and answering wrongly
/// either way would let a caller discard live work. A consumer learns the
/// channel is closed from [`Consume::recv`] returning [`Disconnected`].
pub trait Consume<T>: Send + Sync {
    fn recv(&self) -> Result<T, Disconnected>;
    fn try_recv(&self) -> Option<T>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::port::transport::{InProcess, Transport};

    /// The protocol tests run against the production transport, so they cover
    /// the two ends against one another rather than a stand-in that could
    /// quietly disagree with it.
    fn pair() -> (Outbox<u32>, Mailbox<u32>) {
        InProcess.channel()
    }

    #[test]
    fn messages_arrive_in_the_order_they_were_posted() {
        let (outbox, mailbox) = pair();
        for value in 0..3 {
            outbox.send(value).expect("consumer is present");
        }
        let received: Vec<u32> = std::iter::from_fn(|| mailbox.try_recv()).collect();
        assert_eq!(received, [0, 1, 2]);
    }

    /// The poll path must never block: a UI thread that calls it on an idle
    /// channel has to get `None` and carry on with the event loop.
    #[test]
    fn try_recv_on_an_idle_channel_reports_nothing() {
        let (_outbox, mailbox) = pair();
        assert!(mailbox.try_recv().is_none());
    }

    #[test]
    fn recv_waits_for_a_message_that_has_not_been_posted_yet() {
        let (outbox, mailbox) = pair();
        let producer = std::thread::spawn(move || {
            outbox.send(7).expect("consumer is present");
        });
        assert_eq!(mailbox.recv().expect("producer is alive"), 7);
        producer.join().expect("producer thread finished");
    }

    #[test]
    fn dropping_the_outbox_drains_before_reporting_disconnection() {
        let (outbox, mailbox) = pair();
        outbox.send(1).expect("consumer is present");
        drop(outbox);
        assert_eq!(mailbox.recv().expect("a message is still queued"), 1);
        assert_eq!(mailbox.recv(), Err(Disconnected));
    }

    #[test]
    fn sending_to_a_dropped_consumer_hands_the_message_back() {
        let (outbox, mailbox) = pair();
        drop(mailbox);
        assert_eq!(outbox.send(3), Err(Undeliverable(3)));
    }

    /// Several producers may post into one mailbox, which is what lets the
    /// network core, the immediate-scheme path and a worker all feed the same
    /// queue without coordinating first.
    #[test]
    fn every_clone_of_an_outbox_reaches_the_same_mailbox() {
        let (outbox, mailbox) = pair();
        let mut producers = Vec::new();
        for value in 0..4u32 {
            let outbox = outbox.clone();
            producers.push(std::thread::spawn(move || {
                outbox.send(value).expect("consumer is present");
            }));
        }
        for producer in producers {
            producer.join().expect("producer thread finished");
        }
        let mut received: Vec<u32> = std::iter::from_fn(|| mailbox.try_recv()).collect();
        received.sort_unstable();
        assert_eq!(received, [0, 1, 2, 3]);
    }

    /// Both ends are shared across threads by design, so this bound is part of
    /// the contract rather than an implementation detail.
    #[test]
    fn ends_are_shareable_across_threads() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<Outbox<u32>>();
        assert_sync::<Outbox<u32>>();
        assert_send::<Mailbox<u32>>();
        assert_sync::<Mailbox<u32>>();
    }
}
