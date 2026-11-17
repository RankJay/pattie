//! 16-bit PCM through a Ring: what a PCM16 read or write does to the frames.

// The Ring in 16-bit PCM: the app side stays float (ADR 0005).

use pattie_conformance::Ring;

#[test]
fn frames_the_app_writes_come_out_of_a_pcm16_read_converted() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.5, -0.25, 1.0, -1.0]);

    let mut out = [9i16; 4];
    ring.system_read_pcm16(&mut out);

    assert_eq!(out, [16384, -8192, 32767, -32768]);
}

#[test]
fn a_pcm16_read_of_an_empty_ring_gives_silence_counted_as_a_gap() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.5]);

    let mut out = [9i16; 3];
    ring.system_read_pcm16(&mut out);

    assert_eq!(
        (out, ring.header().gap_frames, ring.header().read_frames),
        ([16384, 0, 0], 2, 1)
    );
}

#[test]
fn pcm16_frames_the_driver_writes_reach_the_app_as_float() {
    let mut ring = Ring::new(256);

    ring.system_write_pcm16(&[16384, -8192, -32768, 1]);

    assert_eq!(ring.app_read(4), vec![0.5, -0.25, -1.0, 1.0 / 32768.0]);
}

#[test]
fn a_full_ring_drops_new_pcm16_frames_counted_as_overflow() {
    let mut ring = Ring::new(256);
    ring.system_write_pcm16(&[1; 250]);

    ring.system_write_pcm16(&[2; 10]);

    let header = ring.header();
    assert_eq!((header.write_frames, header.overflow_frames), (256, 4));
}
