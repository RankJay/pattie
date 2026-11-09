//! The Linux backend (ADR 0018): the app's own process plays the driver. The Core
//! host (`crate::core_host`) runs the embedded C core, and each Device becomes a
//! PipeWire node that streams through its Ring ([`pipewire`]).

pub(crate) mod pipewire;

use crate::Error;
use crate::core_host::HostTransport;
use std::sync::Arc;

/// PipeWire's graph pulls a whole cycle at once: 1,024 frames (21.3 ms) by
/// default, up to 2,048 at its default maximum. Leading by 960 left every
/// 1,024-frame pull 64 frames short (WSL, 2026-11-08).
const LEAD_FRAMES: u32 = 2_048;

/// The default transport on Linux: the process's core and real PipeWire nodes.
pub(crate) fn transport() -> Result<HostTransport, Error> {
    HostTransport::for_process(Arc::new(pipewire::PipeWireNodes), LEAD_FRAMES)
}
