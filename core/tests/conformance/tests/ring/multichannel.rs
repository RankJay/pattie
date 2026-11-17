//! Stereo and more: every path again with whole frames of two or more channels.

use super::*;

// ---- Formats beyond 48 kHz mono (ADR 0017's settings, made real) ----

#[test]
fn a_stereo_ring_carries_whole_frames_in_order() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 10));

    let mut out = [0.0f32; 20];
    ring.system_read(&mut out);

    assert_eq!(out.to_vec(), stereo(0, 10));
    assert_eq!(ring.system_counters().read, 10, "counters are in frames");
}

#[test]
fn a_stereo_gap_is_whole_silent_frames_counted_once_each() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 3));

    let mut out = [9.0f32; 10]; // five frames asked, three there
    ring.system_read(&mut out);

    assert_eq!(out[..6].to_vec(), stereo(0, 3));
    assert_eq!(out[6..], [0.0; 4]);
    assert_eq!(ring.system_counters().gap, 2);
}

#[test]
fn stereo_audio_wraps_past_the_end_frame_by_frame() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 250));
    ring.system_read(&mut vec![0.0f32; 500]);

    ring.app_write(&stereo(250, 20)); // frames 256-269 wrap to the start
    let mut out = vec![0.0f32; 40];
    ring.system_read(&mut out);

    assert_eq!(out, stereo(250, 20));
}

#[test]
fn a_stereo_speaker_hands_the_app_whole_frames() {
    let mut ring = Ring::with_format(256, 44_100, 2);

    ring.system_write(&stereo(0, 7));

    assert_eq!(ring.app_read(7), stereo(0, 7));
    assert_eq!(ring.system_counters().write, 7);
}

#[test]
fn a_stereo_pcm16_read_converts_every_sample() {
    let mut ring = Ring::with_format(256, 96_000, 2);
    ring.app_write(&[0.5, -0.5, 0.25, -0.25]);

    let mut out = [0i16; 4];
    ring.system_read_pcm16(&mut out);

    assert_eq!(out, [16_384, -16_384, 8_192, -8_192]);
}

#[test]
fn a_stereo_pcm16_write_converts_every_sample() {
    let mut ring = Ring::with_format(256, 48_000, 2);

    ring.system_write_pcm16(&[16_384, -16_384, 8_192, -8_192]);

    assert_eq!(ring.app_read(2), [0.5, -0.5, 0.25, -0.25]);
}

#[test]
fn a_stereo_stale_skip_keeps_whole_frames() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 100));

    ring.system_skip_stale(10);
    let mut out = vec![0.0f32; 20];
    ring.system_read(&mut out);

    assert_eq!(out, stereo(90, 10));
}
