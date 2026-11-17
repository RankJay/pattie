//! Seam A: the control path (ADR 0008).

use pattie_conformance::ffi::*;

use pattie_conformance::{
    DEFAULT_RING_FRAMES, DeviceInfo, Harness, RingMemory, as_bytes, create_device_request,
    stable_id,
};

mod create;
mod device_info;
mod hello;
mod names;
mod remove;

fn greeted_driver() -> (Harness, pattie_conformance::OwnerRef) {
    let mut driver = Harness::new();
    let owner = driver.connect();
    driver.greet(owner);
    (driver, owner)
}

fn send_create(
    driver: &mut Harness,
    owner: pattie_conformance::OwnerRef,
    request: &pattie_create_device_request,
) -> pattie_status {
    driver
        .send(owner, PATTIE_CMD_CREATE_DEVICE, as_bytes(request))
        .0
}
