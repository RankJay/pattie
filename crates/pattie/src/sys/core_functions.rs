// The C core's functions (core/include/pattie_core.h), as the Linux backend calls
// them. The includer brings `c_void` and the types `RawCore`, `RawOwner`, `RawRing`
// and `DeviceInfo` into scope: the SDK's own (sys/mod.rs, device_info.rs), or
// bindgen's in core/tests/conformance/tests/extern_mirror.rs, which checks each signature
// against the header.
unsafe extern "C" {
    pub(crate) fn pattie_core_command(
        core: *mut RawCore,
        owner: *mut RawOwner,
        command: u32,
        input: *const c_void,
        input_len: usize,
        ring_memory: *mut c_void,
        ring_len: usize,
        output: *mut c_void,
        output_cap: usize,
        output_len: *mut usize,
    ) -> u32;
    pub(crate) fn pattie_core_owner_closed(core: *mut RawCore, owner: *mut RawOwner, removed: *mut u32) -> usize;
    pub(crate) fn pattie_core_device_ring(core: *mut RawCore, device_id: u32) -> *mut RawRing;
    pub(crate) fn pattie_core_device_info(core: *const RawCore, device_id: u32, info: *mut DeviceInfo) -> bool;
    pub(crate) fn pattie_ring_channels(ring: *const RawRing) -> u32;
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "only PipeWire's format needs the rate")
    )]
    pub(crate) fn pattie_ring_sample_rate(ring: *const RawRing) -> u32;
    pub(crate) fn pattie_ring_read(ring: *mut RawRing, dst: *mut f32, count: u32);
    pub(crate) fn pattie_ring_write(ring: *mut RawRing, src: *const f32, count: u32);
    pub(crate) fn pattie_ring_publish_read_clock(ring: *mut RawRing, now_ns: u64);
    pub(crate) fn pattie_ring_publish_write_clock(ring: *mut RawRing, now_ns: u64);
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "the PipeWire node drops stale audio; test nodes don't"
        )
    )]
    pub(crate) fn pattie_ring_skip_stale(ring: *mut RawRing, keep_frames: u32) -> u64;
}
