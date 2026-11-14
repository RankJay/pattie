// The C core's functions (core/src/viaduct_core.h), as the Linux backend calls
// them. The includer brings `c_void` and the types `VdCore`, `VdOwner`, `VdRing`
// and `VdDeviceInfo` into scope: the SDK's own (core_ffi.rs, core_types.rs), or
// bindgen's in core/tests/tests/ffi_mirror.rs, which checks each signature
// against the header.
unsafe extern "C" {
    fn vd_core_command(
        core: *mut VdCore,
        owner: *mut VdOwner,
        command: u32,
        input: *const c_void,
        input_len: usize,
        ring_memory: *mut c_void,
        ring_len: usize,
        output: *mut c_void,
        output_cap: usize,
        output_len: *mut usize,
    ) -> u32;
    fn vd_core_owner_closed(core: *mut VdCore, owner: *mut VdOwner, removed: *mut u32) -> usize;
    fn vd_core_device_ring(core: *mut VdCore, device_id: u32) -> *mut VdRing;
    fn vd_core_device_info(core: *const VdCore, device_id: u32, info: *mut VdDeviceInfo) -> bool;
    fn vd_ring_channels(ring: *const VdRing) -> u32;
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "only PipeWire's format needs the rate")
    )]
    fn vd_ring_sample_rate(ring: *const VdRing) -> u32;
    fn vd_ring_read(ring: *mut VdRing, dst: *mut f32, count: u32);
    fn vd_ring_write(ring: *mut VdRing, src: *const f32, count: u32);
    fn vd_ring_publish_read_clock(ring: *mut VdRing, now_ns: u64);
    fn vd_ring_publish_write_clock(ring: *mut VdRing, now_ns: u64);
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "the PipeWire node drops stale audio; test nodes don't"
        )
    )]
    fn vd_ring_skip_stale(ring: *mut VdRing, keep_frames: u32) -> u64;
}
