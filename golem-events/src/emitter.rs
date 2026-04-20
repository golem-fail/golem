use crate::channel::EventSender;
use crate::{DeviceId, EventKind, SubstepEvent};

/// Scoped event emitter for a single device execution.
/// Passed through ExecutionContext so all action handlers can emit events.
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
