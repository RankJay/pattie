//! Seam B: the Ring (ADR 0007, ADR 0008).

use viaduct_driver_core_tests::Ring;

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
