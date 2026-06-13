use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Instant, SystemTime};

use tokio::sync::broadcast;

use crate::{DeviceId, Event, EventKind};

/// Channel capacity. Events are small (~100 bytes) and consumers drain fast.
const CHANNEL_CAPACITY: usize = 4096;

/// Create a new event channel. Returns (sender, receiver_factory).
pub fn event_channel() -> (EventSender, EventSubscriptions) {
    let (tx, _rx) = broadcast::channel(CHANNEL_CAPACITY);
    let seq = Arc::new(AtomicU64::new(0));
    (
        EventSender { tx: tx.clone(), seq: seq.clone() },
        EventSubscriptions { tx },
    )
}

/// Cloneable sender — one clone per device task.
#[derive(Clone)]
pub struct EventSender {
    tx: broadcast::Sender<Event>,
    seq: Arc<AtomicU64>,
}

impl EventSender {
    /// Emit an event tagged with a device ID.
    ///
    /// Captures both clocks: `timestamp` (monotonic, for duration math) and
    /// `wall_time` (wall-clock, for display). Both are captured at emit,
    /// not send — the consumer sees the moment the producer called `emit`.
    pub fn emit(&self, device_id: DeviceId, kind: EventKind) {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let event = Event {
            seq,
            device_id,
            timestamp: Instant::now(),
            wall_time: SystemTime::now(),
            kind,
        };
        // Ignore send errors (no receivers = lag).
        let _ = self.tx.send(event);
    }
}

/// Factory for creating new broadcast receivers.
pub struct EventSubscriptions {
    tx: broadcast::Sender<Event>,
}

impl EventSubscriptions {
    /// Subscribe a new consumer. Each consumer gets its own receiver.
    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DeviceId, EventKind};

    #[test]
    fn event_channel_creates_working_pair() {
        let (sender, subs) = event_channel();
        let _rx = subs.subscribe();
        // Emitting should not panic even with no prior subscribers
        sender.emit(DeviceId("dev1".into()), EventKind::SuiteStarted { flow_count: 1 });
    }

    #[tokio::test]
    async fn emit_sends_event_receivable_via_subscribe() {
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();

        sender.emit(
            DeviceId("pixel_7".into()),
            EventKind::FlowStarted { flow_name: "login".into(), os_major: 0 , repeat: None},
        );

        let event = rx.recv().await.expect("SHALL receive the emitted event");
        assert!(
            matches!(event.kind, EventKind::FlowStarted { ref flow_name, .. } if flow_name == "login"),
            "SHALL carry the correct EventKind"
        );
    }

    #[tokio::test]
    async fn sequence_numbers_increment_monotonically() {
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let dev = DeviceId("dev".into());

        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 1 });
        sender.emit(dev.clone(), EventKind::FlowStarted { flow_name: "a".into(), os_major: 0 , repeat: None});
        sender.emit(dev.clone(), EventKind::SuiteFinished { duration_ms: 100, passed: 1, failed: 0, skipped: 0 });

        let e0 = rx.recv().await.expect("SHALL receive event 0");
        let e1 = rx.recv().await.expect("SHALL receive event 1");
        let e2 = rx.recv().await.expect("SHALL receive event 2");

        assert_eq!(e0.seq, 0, "SHALL start at 0");
        assert_eq!(e1.seq, 1, "SHALL increment to 1");
        assert_eq!(e2.seq, 2, "SHALL increment to 2");
        assert!(e0.seq < e1.seq && e1.seq < e2.seq, "SHALL be strictly monotonic");
    }

    #[tokio::test]
    async fn multiple_subscribers_each_get_all_events() {
        let (sender, subs) = event_channel();
        let mut rx1 = subs.subscribe();
        let mut rx2 = subs.subscribe();

        sender.emit(
            DeviceId("dev".into()),
            EventKind::FlowStarted { flow_name: "f1".into(), os_major: 0 , repeat: None},
        );
        sender.emit(
            DeviceId("dev".into()),
            EventKind::FlowFinished { flow_name: "f1".into(), success: true, duration_ms: 50, seed: 0, os_major: 0 , code: None, repeat: None},
        );

        // Both subscribers SHALL receive both events
        let r1_a = rx1.recv().await.expect("subscriber-1 SHALL receive event-0");
        let r1_b = rx1.recv().await.expect("subscriber-1 SHALL receive event-1");
        let r2_a = rx2.recv().await.expect("subscriber-2 SHALL receive event-0");
        let r2_b = rx2.recv().await.expect("subscriber-2 SHALL receive event-1");

        assert_eq!(r1_a.seq, 0);
        assert_eq!(r1_b.seq, 1);
        assert_eq!(r2_a.seq, 0);
        assert_eq!(r2_b.seq, 1);
    }

    #[tokio::test]
    async fn device_id_is_correctly_tagged_on_events() {
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();

        sender.emit(
            DeviceId("ios/iPhone 15 Pro".into()),
            EventKind::FlowStarted { flow_name: "test".into(), os_major: 0 , repeat: None},
        );
        sender.emit(
            DeviceId("android/Pixel 7".into()),
            EventKind::FlowStarted { flow_name: "test".into(), os_major: 0 , repeat: None},
        );

        let e1 = rx.recv().await.expect("SHALL receive first event");
        let e2 = rx.recv().await.expect("SHALL receive second event");

        assert_eq!(e1.device_id, DeviceId("ios/iPhone 15 Pro".into()),
            "SHALL tag first event with iOS device ID");
        assert_eq!(e2.device_id, DeviceId("android/Pixel 7".into()),
            "SHALL tag second event with Android device ID");
    }

    #[tokio::test]
    async fn sequence_shared_across_cloned_senders() {
        let (sender, subs) = event_channel();
        let sender2 = sender.clone();
        let mut rx = subs.subscribe();

        sender.emit(DeviceId("a".into()), EventKind::SuiteStarted { flow_count: 1 });
        sender2.emit(DeviceId("b".into()), EventKind::SuiteStarted { flow_count: 1 });

        let e0 = rx.recv().await.expect("SHALL receive event from sender");
        let e1 = rx.recv().await.expect("SHALL receive event from sender2");

        assert_eq!(e0.seq, 0);
        assert_eq!(e1.seq, 1, "SHALL share sequence counter across clones");
    }
}
