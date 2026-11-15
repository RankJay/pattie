// The C core's functions (core/src/pattie_core.h), as the Linux backend calls
// them. The includer brings `c_void` and the types `PattieCore`, `PattieOwner`, `PattieRing`
// and `PattieDeviceInfo` into scope: the SDK's own (core_ffi.rs, core_types.rs), or
// bindgen's in core/tests/tests/ffi_mirror.rs, which checks each signature
// against the header.
unsafe extern "C" {
    fn pattie_core_command(
        core: *mut PattieCore,
        owner: *mut PattieOwner,
        command: u32,
        input: *const c_void,
        input_len: usize,
        ring_memory: *mut c_void,
        ring_len: usize,
        output: *mut c_void,
        output_cap: usize,
        output_len: *mut usize,
    ) -> u32;
    fn pattie_core_owner_closed(core: *mut PattieCore, owner: *mut PattieOwner, removed: *mut u32) -> usize;
    fn pattie_core_device_ring(core: *mut PattieCore, device_id: u32) -> *mut PattieRing;
    fn pattie_core_device_info(core: *const PattieCore, device_id: u32, info: *mut PattieDeviceInfo) -> bool;
    fn pattie_ring_channels(ring: *const PattieRing) -> u32;
    #[cfg_attr(
        not(target_os = "linux"),
        allow(dead_code, reason = "only PipeWire's format needs the rate")
    )]
    fn pattie_ring_sample_rate(ring: *const PattieRing) -> u32;
    fn pattie_ring_read(ring: *mut PattieRing, dst: *mut f32, count: u32);
    fn pattie_ring_write(ring: *mut PattieRing, src: *const f32, count: u32);
    fn pattie_ring_publish_read_clock(ring: *mut PattieRing, now_ns: u64);
    fn pattie_ring_publish_write_clock(ring: *mut PattieRing, now_ns: u64);
    #[cfg_attr(
        not(target_os = "linux"),
        allow(
            dead_code,
            reason = "the PipeWire node drops stale audio; test nodes don't"
        )
    )]
    fn pattie_ring_skip_stale(ring: *mut PattieRing, keep_frames: u32) -> u64;
}
