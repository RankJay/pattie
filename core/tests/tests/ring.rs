//! Seam B: the Ring (ADR 0007, ADR 0008).

use viaduct_core_tests::Ring;
use viaduct_core_tests::ffi::*;

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

#[test]
fn frames_the_driver_writes_reach_the_app() {
    let mut ring = Ring::new(256);

    ring.driver_write(&[0.125, -0.25]);

    assert_eq!(ring.app_read(2), vec![0.125, -0.25]);
}

#[test]
fn writing_into_a_full_ring_drops_the_new_frames_counted_as_overflow() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[1.0; 256]);

    ring.driver_write(&[2.0; 4]);

    assert_eq!(
        (ring.app_read(300), ring.header().overflow_frames),
        (vec![1.0; 256], 4)
    );
}

#[test]
fn space_freed_by_the_app_is_written_again() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[1.0; 256]);
    ring.app_read(2);

    ring.driver_write(&[2.0; 3]); // room for 2; one more overflows

    assert_eq!(ring.header().overflow_frames, 1);
}

#[test]
fn an_app_read_counter_moved_backwards_never_lets_the_driver_overwrite_unread_audio() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[1.0; 256]);
    ring.app_read(256);
    ring.driver_write(&[1.0; 100]);
    ring.app_header_mut().read_frames = 0; // claims 356 unread frames in a 256 Ring

    ring.driver_write(&[2.0; 5]);

    assert_eq!(ring.header().overflow_frames, 5);
}

#[test]
fn microphone_audio_wraps_cleanly_past_the_end_of_the_buffer() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.0; 250]);
    ring.driver_read(&mut [0.0; 250]);
    let numbered: Vec<f32> = (0..10).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 250..259 straddle the end

    let mut out = [9.0f32; 10];
    ring.driver_read(&mut out);

    assert_eq!(out.to_vec(), numbered);
}

#[test]
fn speaker_audio_wraps_cleanly_past_the_end_of_the_buffer() {
    let mut ring = Ring::new(256);
    ring.driver_write(&[0.0; 250]);
    ring.app_read(250);
    let numbered: Vec<f32> = (0..10).map(|n| n as f32).collect();

    ring.driver_write(&numbered);

    assert_eq!(ring.app_read(10), numbered);
}

#[test]
fn a_recording_start_drops_stale_audio_keeping_the_newest() {
    let mut ring = Ring::new(4096);
    let numbered: Vec<f32> = (0..3000).map(|n| n as f32).collect();
    ring.app_write(&numbered); // written while nobody was recording

    ring.driver_skip_stale(480);
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [2520.0]); // 3000 - 480: the newest 480 frames remain
}

#[test]
fn a_backlog_within_the_keep_is_left_alone() {
    let mut ring = Ring::new(4096);
    // Numbered from 1, so the oldest frame can't be mistaken for silence.
    let numbered: Vec<f32> = (1..=100).map(|n| n as f32).collect();
    ring.app_write(&numbered);

    ring.driver_skip_stale(480);
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [1.0]);
}

#[test]
fn dropped_stale_audio_is_neither_a_gap_nor_overflow_and_the_keep_stays_buffered() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 3000]);

    ring.driver_skip_stale(480);

    let header = ring.header();
    let buffered = header.write_frames - header.read_frames;
    assert_eq!(
        (header.gap_frames, header.overflow_frames, buffered),
        (0, 0, 480)
    );
}

#[test]
fn a_writer_counter_moved_backwards_never_pulls_the_read_position_back() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 10]);
    ring.driver_read(&mut [0.0; 10]);
    ring.app_header_mut().write_frames = 0; // counters must only grow

    ring.driver_skip_stale(480);

    assert_eq!(ring.header().read_frames, 10);
}

#[test]
fn keeping_zero_frames_drops_everything_unread() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 3000]);

    ring.driver_skip_stale(0);

    let header = ring.header();
    assert_eq!(header.write_frames - header.read_frames, 0);
}

#[test]
fn a_keep_at_or_above_capacity_never_drops_a_backlog_the_ring_can_hold() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (1..=256).map(|n| n as f32).collect();
    ring.app_write(&numbered); // exactly full

    ring.driver_skip_stale(1_000_000);
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [1.0]);
}

#[test]
fn a_backlog_beyond_capacity_keeps_the_newest_like_the_overrun_skip() {
    let mut ring = Ring::new(256);
    let numbered: Vec<f32> = (1..=300).map(|n| n as f32).collect();
    ring.app_write(&numbered); // frames 257..300 overwrote 1..44

    ring.driver_skip_stale(1_000_000); // keep is clamped to the capacity
    let mut first = [9.0f32; 1];
    ring.driver_read(&mut first);

    assert_eq!(first, [45.0]); // the oldest intact frame, as the overrun skip gives
}

#[test]
fn a_microphone_pass_publishes_its_read_position_and_time_on_the_readers_line() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 960]);
    ring.driver_read(&mut [0.0; 480]);

    ring.driver_publish_read_clock(1_234_567_890);

    let clock = ring.header().reader_clock;
    assert_eq!((clock.frames, clock.ns), (480, 1_234_567_890));
    assert_eq!(clock.seq % 2, 0, "even: not mid-write");
    assert_eq!(
        ring.header().writer_clock.seq,
        0,
        "the app's line is untouched"
    );
}

#[test]
fn a_speaker_pass_publishes_its_write_position_and_time_on_the_writers_line() {
    let mut ring = Ring::new(4096);
    ring.driver_write(&[0.25; 480]);

    ring.driver_publish_write_clock(42);

    let clock = ring.header().writer_clock;
    assert_eq!((clock.frames, clock.ns, clock.seq % 2), (480, 42, 0));
    assert_eq!(ring.header().reader_clock.seq, 0);
}

#[test]
fn each_publish_moves_the_sequence_on_by_two_whatever_the_app_wrote_there() {
    let mut ring = Ring::new(4096);
    ring.driver_publish_read_clock(10);
    let first = ring.header().reader_clock.seq;
    ring.app_header_mut().reader_clock.seq = 12_345; // the app scribbles on it

    ring.driver_publish_read_clock(20);

    assert_eq!(ring.header().reader_clock.seq, first + 2);
}

/// The SDK's seqlock reader (`viaduct::abi::read_clock`) on what the C core
/// published: the two sides of ADR 0022's protocol agree.
fn sdk_reads(clock: &vd_ring_clock) -> Option<(u64, u64)> {
    let clock = (clock as *const vd_ring_clock).cast::<viaduct::abi::RingClock>();
    // SAFETY: a live, aligned clock pair with the same layout (abi_mirror.rs),
    // stored to by nobody else while this test reads it.
    unsafe { viaduct::abi::read_clock(clock) }
}

#[test]
fn the_sdk_reads_back_the_pair_the_core_published() {
    let mut ring = Ring::new(4096);
    let never = sdk_reads(&ring.header().reader_clock);
    ring.app_write(&[0.5; 960]);
    ring.driver_read(&mut [0.0; 480]);
    ring.driver_publish_read_clock(777);

    let published = sdk_reads(&ring.header().reader_clock);

    assert_eq!((never, published), (None, Some((480, 777))));
}

#[test]
fn the_sdk_gives_up_on_a_pair_stuck_mid_write() {
    let mut ring = Ring::new(4096);
    ring.driver_publish_read_clock(777);
    ring.app_header_mut().reader_clock.seq += 1; // odd: as if the driver died mid-write

    assert_eq!(sdk_reads(&ring.header().reader_clock), None);
}

// ---- Formats beyond 48 kHz mono (ADR 0017's settings, made real) ----

/// `frames` stereo frames whose samples say which frame and channel they are:
/// frame n is (n + 0.25, -(n + 0.75)), never zero.
fn stereo(first: u32, frames: u32) -> Vec<f32> {
    (first..first + frames)
        .flat_map(|n| [n as f32 + 0.25, -(n as f32 + 0.75)])
        .collect()
}

#[test]
fn a_stereo_ring_carries_whole_frames_in_order() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 10));

    let mut out = [0.0f32; 20];
    ring.driver_read(&mut out);

    assert_eq!(out.to_vec(), stereo(0, 10));
    assert_eq!(ring.driver_counters().read, 10, "counters are in frames");
}

#[test]
fn a_stereo_gap_is_whole_silent_frames_counted_once_each() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 3));

    let mut out = [9.0f32; 10]; // five frames asked, three there
    ring.driver_read(&mut out);

    assert_eq!(out[..6].to_vec(), stereo(0, 3));
    assert_eq!(out[6..], [0.0; 4]);
    assert_eq!(ring.driver_counters().gap, 2);
}

#[test]
fn stereo_audio_wraps_past_the_end_frame_by_frame() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 250));
    ring.driver_read(&mut vec![0.0f32; 500]);

    ring.app_write(&stereo(250, 20)); // frames 256-269 wrap to the start
    let mut out = vec![0.0f32; 40];
    ring.driver_read(&mut out);

    assert_eq!(out, stereo(250, 20));
}

#[test]
fn a_stereo_speaker_hands_the_app_whole_frames() {
    let mut ring = Ring::with_format(256, 44_100, 2);

    ring.driver_write(&stereo(0, 7));

    assert_eq!(ring.app_read(7), stereo(0, 7));
    assert_eq!(ring.driver_counters().write, 7);
}

#[test]
fn a_stereo_pcm16_read_converts_every_sample() {
    let mut ring = Ring::with_format(256, 96_000, 2);
    ring.app_write(&[0.5, -0.5, 0.25, -0.25]);

    let mut out = [0i16; 4];
    ring.driver_read_pcm16(&mut out);

    assert_eq!(out, [16_384, -16_384, 8_192, -8_192]);
}

#[test]
fn a_stereo_pcm16_write_converts_every_sample() {
    let mut ring = Ring::with_format(256, 48_000, 2);

    ring.driver_write_pcm16(&[16_384, -16_384, 8_192, -8_192]);

    assert_eq!(ring.app_read(2), [0.5, -0.5, 0.25, -0.25]);
}

#[test]
fn a_stereo_stale_skip_keeps_whole_frames() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_write(&stereo(0, 100));

    ring.driver_skip_stale(10);
    let mut out = vec![0.0f32; 20];
    ring.driver_read(&mut out);

    assert_eq!(out, stereo(90, 10));
}

#[test]
fn the_three_rates_and_one_to_eight_channels_are_accepted() {
    let accepted: Vec<_> = [44_100, 48_000, 96_000]
        .into_iter()
        .flat_map(|rate| (1..=8u16).map(move |channels| (rate, channels)))
        .filter(|&(rate, channels)| {
            let len = Ring::bytes_for_format(256, channels);
            Ring::attach_format(256, rate, channels, len).is_err()
        })
        .collect();

    assert_eq!(accepted, [], "refused");
}

#[test]
fn zero_settings_mean_48_khz_mono() {
    let ring = Ring::with_format(256, 0, 0);

    assert_eq!(
        (ring.driver_sample_rate(), ring.driver_channels()),
        (48_000, 1)
    );
}

#[test]
fn the_ring_reports_the_rate_and_channels_it_attached_with() {
    let reported: Vec<_> = [(44_100, 2), (96_000, 8), (48_000, 6)]
        .into_iter()
        .map(|(rate, channels)| {
            let ring = Ring::with_format(256, rate, channels);
            (ring.driver_sample_rate(), ring.driver_channels())
        })
        .collect();

    assert_eq!(reported, [(44_100, 2), (96_000, 8), (48_000, 6)]);
}

#[test]
fn other_rates_and_channel_counts_are_rejected() {
    let refused: Vec<_> = [
        (22_050, 1),
        (44_101, 2),
        (192_000, 2),
        (48_000, 9),
        (48_000, 64),
    ]
    .into_iter()
    .map(|(rate, channels)| {
        let len = Ring::bytes_for_format(256, channels);
        Ring::attach_format(256, rate, channels, len).err()
    })
    .collect();

    assert_eq!(refused, [Some(VD_ERR_BAD_RING); 5]);
}

#[test]
fn memory_sized_for_mono_is_too_small_for_stereo() {
    let attached = Ring::attach_format(256, 48_000, 2, Ring::bytes_for(256));

    assert_eq!(attached.err(), Some(VD_ERR_BAD_RING));
}

#[test]
fn the_largest_ring_is_eight_channels_at_the_most_frames() {
    assert_eq!(
        VD_RING_MAX_BYTES as usize,
        Ring::bytes_for_format(VD_RING_MAX_FRAMES, VD_RING_MAX_CHANNELS as u16)
    );
}

#[test]
fn app_rewriting_channels_after_attach_changes_nothing() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_header_mut().channels = 8; // the driver keeps its own copy

    ring.driver_write(&stereo(0, 2));

    assert_eq!(ring.driver_channels(), 2);
    assert_eq!(ring.driver_counters().write, 2);
}

#[test]
fn a_recording_start_says_how_many_stale_frames_it_dropped() {
    let mut ring = Ring::new(4096);
    ring.app_write(&vec![0.5f32; 3000]);

    let skipped = ring.driver_skip_stale_counted(480);
    let again = ring.driver_skip_stale_counted(480);

    assert_eq!((skipped, again), (2_520, 0));
}

#[test]
fn the_losses_are_the_drivers_gap_and_overflow_totals() {
    let mut microphone = Ring::new(256);
    microphone.app_write(&[0.5; 100]);
    let mut out = [0.0f32; 160];
    microphone.driver_read(&mut out); // 60 frames short: Gap
    let mut speaker = Ring::new(256);
    speaker.driver_write(&[0.5; 300]); // 44 frames past the capacity: Overflow

    assert_eq!(
        (microphone.driver_losses(), speaker.driver_losses()),
        ((60, 0), (0, 44))
    );
}
