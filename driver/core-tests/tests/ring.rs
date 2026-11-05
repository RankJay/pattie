//! Seam B: the Ring (ADR 0007, ADR 0008).

use viaduct_driver_core_tests::Ring;
use viaduct_driver_core_tests::ffi::*;

#[test]
fn memory_too_small_for_the_claimed_capacity_is_rejected() {
    let one_frame_short = Ring::bytes_for(256) - 4;

    let attached = Ring::attach(256, one_frame_short);

    assert_eq!(attached.err(), Some(VD_ERR_BAD_RING));
}

#[test]
fn memory_too_small_for_the_header_is_rejected() {
    let attached = Ring::attach(256, 100);

    assert_eq!(attached.err(), Some(VD_ERR_BAD_RING));
}

#[test]
fn capacity_outside_the_allowed_powers_of_two_is_rejected() {
    let bad = [0, 128, 300, 1000, 131_072];

    let results: Vec<_> = bad
        .iter()
        .map(|&capacity| Ring::attach(capacity, Ring::bytes_for(capacity)).err())
        .collect();

    assert_eq!(results, vec![Some(VD_ERR_BAD_RING); bad.len()]);
}

#[test]
fn app_rewriting_capacity_after_attach_changes_nothing() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.0; 256]);
    ring.driver_read(&mut [0.0; 256]); // next frames wrap to the buffer's start
    ring.app_write(&[0.5, 0.75]);
    ring.app_header_mut().capacity_frames = 1 << 30; // far beyond the real memory

    let mut out = [9.0f32; 2];
    ring.driver_read(&mut out);

    assert_eq!(out, [0.5, 0.75]);
}

#[test]
fn after_the_app_overruns_its_ring_read_resumes_at_the_oldest_intact_frame() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (0..300).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 256..299 overwrote frames 0..43

    let mut out = [9.0f32; 1];
    ring.driver_read(&mut out);

    assert_eq!(out, [44.0]);
}

#[test]
fn a_writer_counter_moved_backwards_reads_as_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);
    ring.app_write(&[1.0, 2.0, 3.0, 4.0]);
    ring.driver_read(&mut [0.0; 4]);
    ring.app_header_mut().write_frames = 0; // counters must only grow

    let mut out = [9.0f32; 2];
    ring.driver_read(&mut out);

    assert_eq!((out, ring.header().gap_frames), ([0.0; 2], 2));
}

#[test]
fn frames_the_app_writes_come_back_out_of_read() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.25, -0.5, 1.0]);

    let mut out = [9.0f32; 3];
    ring.driver_read(&mut out);

    assert_eq!(out, [0.25, -0.5, 1.0]);
}

#[test]
fn reading_an_empty_ring_gives_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);

    let mut out = [9.0f32; 4];
    ring.driver_read(&mut out);

    assert_eq!((out, ring.header().gap_frames), ([0.0; 4], 4));
}
