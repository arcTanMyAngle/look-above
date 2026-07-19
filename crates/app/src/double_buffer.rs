//! A single-slot, latest-wins hand-off from one producer thread to one consumer thread.
//!
//! The simulation worker ([`crate::simulation`]) produces a fresh
//! [`RenderFeed`](look_above_core::sim::RenderFeed) every frame on a worker thread; the render
//! thread consumes the most recent one at the start of each frame it draws. This is ADR-002's
//! "results written into the inactive render buffer, swapped atomically at frame start; the
//! render thread never computes any of the above" (high-fidelity-flight-visualization skill).
//!
//! It is deliberately *not* a queue. The producer and consumer run at independent rates, and a
//! feed the consumer never reads has no value once a newer one exists — so [`Producer::publish`]
//! overwrites any unconsumed feed rather than buffering it (latest-wins). Conversely, on a frame
//! where the producer has not published anything new, [`Consumer::take_latest`] returns `None`
//! and the render thread keeps showing the "front" buffer it last took, so the picture never
//! blanks between publishes. Those two held buffers — the consumer's current one and the one in
//! the slot — are the two of the double buffer.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The producer half of a [`channel`] pair — see the module doc. Neither half is `Clone`: the
/// slot has exactly one producer (the sim thread) and one consumer (the render thread).
#[derive(Debug)]
pub struct Producer<T> {
    slot: Arc<Mutex<Option<T>>>,
}

/// The consumer half of a [`channel`] pair — see the module doc.
#[derive(Debug)]
pub struct Consumer<T> {
    slot: Arc<Mutex<Option<T>>>,
}

/// Creates a connected producer/consumer pair over a fresh, empty slot.
pub fn channel<T>() -> (Producer<T>, Consumer<T>) {
    let slot = Arc::new(Mutex::new(None));
    (
        Producer {
            slot: Arc::clone(&slot),
        },
        Consumer { slot },
    )
}

impl<T> Producer<T> {
    /// Publishes `value` as the latest, discarding any previously published value the consumer
    /// has not yet taken (latest-wins).
    pub fn publish(&self, value: T) {
        *lock(&self.slot) = Some(value);
    }
}

impl<T> Consumer<T> {
    /// Takes the most recently published value, or `None` if nothing has been published since
    /// the last take — in which case the caller keeps whatever it last held.
    pub fn take_latest(&self) -> Option<T> {
        lock(&self.slot).take()
    }
}

/// Locks the shared slot, recovering the guard rather than propagating a poison panic.
///
/// The slot holds only plain data (an `Option<T>`), and both operations on it — a move-assign
/// and a `take` — are panic-free, so a poisoned lock can arise only if one side panicked for an
/// unrelated reason while holding the guard. Even then the worst the other side sees is a stale
/// (never a torn) value, so carrying on with it beats taking down the render thread too.
fn lock<T>(slot: &Mutex<T>) -> MutexGuard<'_, T> {
    slot.lock().unwrap_or_else(PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn take_before_any_publish_is_none() {
        let (_producer, consumer) = channel::<i32>();
        assert_eq!(consumer.take_latest(), None);
    }

    #[test]
    fn a_published_value_is_taken_exactly_once() {
        let (producer, consumer) = channel();
        producer.publish(7);
        assert_eq!(consumer.take_latest(), Some(7));
        // Nothing new since: the consumer keeps its own held value, it does not re-take.
        assert_eq!(consumer.take_latest(), None);
    }

    #[test]
    fn publishing_twice_without_a_take_keeps_only_the_latest() {
        let (producer, consumer) = channel();
        producer.publish(1);
        producer.publish(2);
        // The first, unconsumed, value was overwritten — a stale frame has no value once a
        // newer one exists.
        assert_eq!(consumer.take_latest(), Some(2));
        assert_eq!(consumer.take_latest(), None);
    }

    #[test]
    fn publishes_and_takes_interleave() {
        let (producer, consumer) = channel();
        producer.publish("a");
        assert_eq!(consumer.take_latest(), Some("a"));
        producer.publish("b");
        producer.publish("c");
        assert_eq!(consumer.take_latest(), Some("c"));
        assert_eq!(consumer.take_latest(), None);
    }
}
