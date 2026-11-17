//! What the SDK needs from the OS outside a Backend: the clock, the doorbell, and
//! thread priority. Each facility names the OS only in its own `win` and `linux` files.

pub(crate) mod clock;
pub(crate) mod doorbell;
pub(crate) mod pro_audio;
