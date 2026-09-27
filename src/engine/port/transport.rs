//! Where a channel's messages physically travel.
//!
//! [`InProcess`] is the only implementation today. A shared-memory or socket
//! one would have to satisfy the same protocol tests — see the tests in
//! [`crate::engine::port::mailbox`].

use super::mailbox::{Consume, Disconnected, Mailbox, Outbox, Produce, Undeliverable};

/// Creates the two ends of a one-way message channel.
pub trait Transport: std::fmt::Debug + Send + Sync {
    /// Pairs a producer with a consumer.
    ///
    /// The channel is unbounded, matching the queues it replaces: backpressure
    /// would change the caller's timing, and a transport that needs it has to
    /// apply it inside the message type instead.
    fn channel<T: Send + 'static>(&self) -> (Outbox<T>, Mailbox<T>);
}

/// Carries messages between threads of this process, over `std::sync::mpsc`.
///
/// Zero-sized on purpose: the placement is a property of the type, not a field
/// someone can swap out at runtime. A remote peer gets its own transport type
/// with a connection in it, which keeps "where does this run" visible in the
/// type rather than in a value that could be wrong at a call site.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InProcess;

impl Transport for InProcess {
    fn channel<T: Send + 'static>(&self) -> (Outbox<T>, Mailbox<T>) {
        let (sender, receiver) = std::sync::mpsc::channel();
        (
            Outbox::of(sender),
            Mailbox::of(Receiver(std::sync::Mutex::new(receiver))),
        )
    }
}

impl<T: Send + 'static> Produce<T> for std::sync::mpsc::Sender<T> {
    // Fully qualified: an unqualified `self.send(..)` would resolve back to
    // this impl and recurse forever.
    fn send(&self, message: T) -> Result<(), Undeliverable<T>> {
        std::sync::mpsc::Sender::send(self, message).map_err(|error| Undeliverable(error.0))
    }
}

/// Wraps `mpsc::Receiver` so it can sit behind a `Sync` trait object.
///
/// `Receiver` is `Send` but not `Sync`, and a mailbox has to be shareable for
/// the owner to stay movable between threads.
struct Receiver<T>(std::sync::Mutex<std::sync::mpsc::Receiver<T>>);

impl<T: Send + 'static> Consume<T> for Receiver<T> {
    fn recv(&self) -> Result<T, Disconnected> {
        // Held across the park. A second thread reading this same mailbox would
        // block on the lock rather than spin, which is the intended answer:
        // one consumer per mailbox.
        self.0
            .lock()
            .map_err(|_| Disconnected)?
            .recv()
            .map_err(|_| Disconnected)
    }

    fn try_recv(&self) -> Option<T> {
        self.0.lock().ok()?.try_recv().ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transport must be usable from another thread, otherwise it cannot
    /// describe a channel between the UI thread and a worker.
    #[test]
    fn transport_is_shareable_across_threads() {
        fn assert_send<T: Send>() {}
        fn assert_sync<T: Sync>() {}
        assert_send::<InProcess>();
        assert_sync::<InProcess>();
    }

    /// A transport is chosen once, where the two ends are paired; both ends
    /// keep working after it is gone.
    #[test]
    fn a_channel_outlives_the_transport_that_made_it() {
        let (outbox, mailbox) = {
            let transport = InProcess;
            transport.channel::<&'static str>()
        };
        outbox.send("hello").expect("consumer is present");
        assert_eq!(mailbox.recv().expect("producer is alive"), "hello");
    }
}
