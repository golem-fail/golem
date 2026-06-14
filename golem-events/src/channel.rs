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

    // 1. Late subscribers SHALL only receive events emitted after they subscribe,
    //    not events that were emitted before the subscription existed.
    #[tokio::test]
    async fn late_subscriber_misses_prior_events() {
        let (sender, subs) = event_channel();
        let dev = DeviceId("dev".into());

        // Emitted before anyone subscribed — dropped (no receivers).
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 1 });

        let mut rx = subs.subscribe();

        // Emitted after subscribe — visible to this receiver.
        sender.emit(dev.clone(), EventKind::SuiteFinished { duration_ms: 1, passed: 1, failed: 0, skipped: 0 });

        let e = rx.recv().await.expect("SHALL receive the post-subscribe event");
        assert_eq!(e.seq, 1, "SHALL skip the pre-subscribe event yet keep its seq slot");
        assert!(
            matches!(e.kind, EventKind::SuiteFinished { .. }),
            "SHALL deliver the event emitted after subscription"
        );
    }

    // 2. The sequence counter SHALL keep advancing even while there are no
    //    receivers (emit drops the event but still bumps seq).
    #[tokio::test]
    async fn seq_advances_without_receivers() {
        let (sender, subs) = event_channel();
        let dev = DeviceId("dev".into());

        // No subscribers: these are dropped but consume seq slots 0 and 1.
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 1 });
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 2 });

        let mut rx = subs.subscribe();
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 3 });

        let e = rx.recv().await.expect("SHALL receive the only live event");
        assert_eq!(e.seq, 2, "seq SHALL have advanced past the two dropped events");
    }

    // 3. Both clocks SHALL be captured at emit (not at send/recv): each event's
    //    timestamp and wall_time fall within a window bracketing the emit calls.
    #[tokio::test]
    async fn emit_captures_both_clocks_at_emit_time() {
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let dev = DeviceId("dev".into());

        // 1. Bracket the emits with locally-sampled clocks.
        let mono_before = Instant::now();
        let wall_before = SystemTime::now();
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 1 });
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 2 });
        let mono_after = Instant::now();
        let wall_after = SystemTime::now();

        let e0 = rx.recv().await.expect("SHALL receive event 0");
        let e1 = rx.recv().await.expect("SHALL receive event 1");

        // 2. Each event's monotonic timestamp SHALL be captured at emit, so it
        //    falls inside the bracketing window — proving emit, not recv, stamps it.
        assert!(
            e0.timestamp >= mono_before && e0.timestamp <= mono_after,
            "event-0 timestamp SHALL be captured during the emit window"
        );
        assert!(
            e1.timestamp >= mono_before && e1.timestamp <= mono_after,
            "event-1 timestamp SHALL be captured during the emit window"
        );

        // 3. Likewise each event's wall_time SHALL fall inside the wall-clock window.
        assert!(
            e0.wall_time >= wall_before && e0.wall_time <= wall_after,
            "event-0 wall_time SHALL be captured during the emit window"
        );
        assert!(
            e1.wall_time >= wall_before && e1.wall_time <= wall_after,
            "event-1 wall_time SHALL be captured during the emit window"
        );

        // 4. Emit order SHALL be preserved on the monotonic clock (same-thread,
        //    no clock adjustment can reorder Instant).
        assert!(
            e1.timestamp >= e0.timestamp,
            "monotonic timestamp SHALL NOT go backwards across emits"
        );
    }

    // 4. Overflowing the broadcast buffer SHALL surface as a Lagged error to a
    //    receiver that did not drain in time; subsequent recv then resumes.
    #[tokio::test]
    async fn slow_receiver_lags_when_buffer_overflows() {
        use tokio::sync::broadcast::error::RecvError;

        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let dev = DeviceId("dev".into());

        // Emit one more than the channel can hold without draining.
        for _ in 0..(CHANNEL_CAPACITY + 1) {
            sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 1 });
        }

        match rx.recv().await {
            Err(RecvError::Lagged(n)) => {
                assert!(n >= 1, "SHALL report at least one skipped message");
            }
            other => panic!("SHALL report Lagged after overflow, got {other:?}"),
        }

        // After observing Lagged the receiver SHALL recover to the oldest retained event.
        let recovered = rx.recv().await.expect("SHALL recover after lag");
        assert!(
            matches!(recovered.kind, EventKind::SuiteStarted { .. }),
            "recovered event SHALL be a retained SuiteStarted"
        );
    }

    // 5. The have-a-receiver -> drop -> none -> re-subscribe transition: the
    //    event emitted during the receiver-less gap SHALL be dropped (not buffered
    //    for a future subscriber), yet SHALL still consume its seq slot.
    #[tokio::test]
    async fn emit_during_receiverless_gap_drops_event_but_keeps_seq() {
        use tokio::sync::broadcast::error::TryRecvError;

        let (sender, subs) = event_channel();
        let rx = subs.subscribe();
        let dev = DeviceId("dev".into());

        // 1. seq 0 emitted while rx is live, then rx is dropped.
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 1 });
        drop(rx);

        // 2. seq 1 emitted with zero live receivers: dropped, but seq still bumped.
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 2 });

        // 3. New subscriber joins, then seq 2 is emitted.
        let mut rx2 = subs.subscribe();
        sender.emit(dev.clone(), EventKind::SuiteStarted { flow_count: 3 });

        // 4. rx2's FIRST event is seq 2 — not the seq 1 from the gap — proving the
        //    gap event was dropped, not retained for late subscribers, while the
        //    seq counter advanced through the gap.
        let e = rx2.recv().await.expect("SHALL receive post-resubscribe event");
        assert_eq!(e.seq, 2, "rx2's first event SHALL be seq 2 (gap event dropped, seq slot consumed)");
        assert!(
            matches!(e.kind, EventKind::SuiteStarted { flow_count: 3 }),
            "rx2's first event SHALL be the post-resubscribe emit, not the gap emit"
        );

        // 5. rx2 SHALL have nothing further buffered — the gap event never reached it.
        match rx2.try_recv() {
            Err(TryRecvError::Empty) => {}
            other => panic!("rx2 SHALL have no further buffered events, got {other:?}"),
        }
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
