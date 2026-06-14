use crate::channel::EventSender;
use crate::{DeviceId, EventKind, SubstepEvent};

/// Scoped event emitter for a single device execution.
/// Passed through ExecutionContext so all action handlers can emit events.
#[derive(Clone)]
pub struct DeviceEmitter {
    sender: EventSender,
    device_id: DeviceId,
}

impl DeviceEmitter {
    pub fn new(sender: EventSender, device_id: DeviceId) -> Self {
        Self { sender, device_id }
    }

    /// Emit a top-level event (step started, flow finished, etc.).
    pub fn emit(&self, kind: EventKind) {
        self.sender.emit(self.device_id.clone(), kind);
    }

    /// Emit a substep detail event.
    pub fn substep(&self, event: SubstepEvent) {
        self.emit(EventKind::Substep(event));
    }

    /// Get the device ID.
    pub fn device_id(&self) -> &DeviceId {
        &self.device_id
    }
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::channel::event_channel;

    // 1. `new` SHALL store the device id, and `device_id()` SHALL return it verbatim.
    #[test]
    fn device_id_accessor_returns_constructed_id() {
        let (sender, _subs) = event_channel();
        let emitter = DeviceEmitter::new(sender, DeviceId("android/Pixel 7".into()));

        assert_eq!(
            emitter.device_id(),
            &DeviceId("android/Pixel 7".into()),
            "device_id() SHALL return the id passed to new"
        );
    }

    // 2. `emit` SHALL deliver the event tagged with the emitter's own device id.
    #[tokio::test]
    async fn emit_tags_event_with_emitter_device_id() {
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let emitter = DeviceEmitter::new(sender, DeviceId("ios/iPhone 15".into()));

        emitter.emit(EventKind::SuiteStarted { flow_count: 3 });

        let event = rx.recv().await.expect("SHALL receive the emitted event");
        assert_eq!(
            event.device_id,
            DeviceId("ios/iPhone 15".into()),
            "emit SHALL tag the event with the emitter's device id"
        );
        assert!(
            matches!(event.kind, EventKind::SuiteStarted { flow_count: 3 }),
            "emit SHALL forward the exact EventKind"
        );
    }

    // 3. `substep` SHALL wrap the SubstepEvent inside EventKind::Substep before emitting.
    #[tokio::test]
    async fn substep_wraps_event_in_substep_kind() {
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let emitter = DeviceEmitter::new(sender, DeviceId("dev".into()));

        emitter.substep(SubstepEvent::Backspace { count: 5 });

        let event = rx.recv().await.expect("SHALL receive the substep event");
        assert_eq!(
            event.device_id,
            DeviceId("dev".into()),
            "substep SHALL tag with the emitter's device id"
        );
        assert!(
            matches!(
                event.kind,
                EventKind::Substep(SubstepEvent::Backspace { count: 5 })
            ),
            "substep SHALL wrap the payload in EventKind::Substep"
        );
    }

    // 4. A cloned emitter SHALL emit into the same shared channel as the original,
    //    so a single subscriber receives events from both, each tagged with the
    //    emitting emitter's own device id.
    #[tokio::test]
    async fn clone_emits_into_shared_channel() {
        let (sender, subs) = event_channel();
        let mut rx = subs.subscribe();
        let original = DeviceEmitter::new(sender, DeviceId("orig".into()));
        let cloned = original.clone();

        original.emit(EventKind::SuiteStarted { flow_count: 1 });
        cloned.emit(EventKind::SuiteFinished {
            duration_ms: 10,
            passed: 1,
            failed: 0,
            skipped: 0,
        });

        // The original's event arrives first on the shared subscriber, tagged "orig".
        let e0 = rx.recv().await.expect("SHALL receive event from original");
        assert_eq!(
            e0.device_id,
            DeviceId("orig".into()),
            "original's event SHALL be tagged with the original device id"
        );
        assert!(
            matches!(e0.kind, EventKind::SuiteStarted { flow_count: 1 }),
            "original SHALL emit its own EventKind"
        );

        // The clone's event reaches the SAME subscriber, also tagged "orig" since
        // clone copies the device id, proving both share one channel.
        let e1 = rx.recv().await.expect("SHALL receive event from clone");
        assert_eq!(
            e1.device_id,
            DeviceId("orig".into()),
            "clone SHALL carry the same device id as the original it was cloned from"
        );
        assert!(
            matches!(
                e1.kind,
                EventKind::SuiteFinished {
                    duration_ms: 10,
                    passed: 1,
                    failed: 0,
                    skipped: 0
                }
            ),
            "clone SHALL emit into the same shared channel as the original"
        );
    }
}
