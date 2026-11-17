//! Clock points: what each Pass publishes, and how the SDK reads the pair back.

use super::*;

#[test]
fn a_microphone_pass_publishes_its_read_position_and_time_on_the_readers_line() {
    let mut ring = Ring::new(4096);
    ring.app_write(&[0.5; 960]);
    ring.system_read(&mut [0.0; 480]);

    ring.system_publish_read_clock(1_234_567_890);

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
    ring.system_write(&[0.25; 480]);

    ring.system_publish_write_clock(42);

    let clock = ring.header().writer_clock;
    assert_eq!((clock.frames, clock.ns, clock.seq % 2), (480, 42, 0));
    assert_eq!(ring.header().reader_clock.seq, 0);
}

#[test]
fn each_publish_moves_the_sequence_on_by_two_whatever_the_app_wrote_there() {
    let mut ring = Ring::new(4096);
    ring.system_publish_read_clock(10);
    let first = ring.header().reader_clock.seq;
    ring.app_header_mut().reader_clock.seq = 12_345; // the app scribbles on it

    ring.system_publish_read_clock(20);

    assert_eq!(ring.header().reader_clock.seq, first + 2);
}

/// The SDK's seqlock reader (`pattie::abi::read_clock`) on what the C core
/// published: the two sides of ADR 0022's protocol agree.
fn sdk_reads(clock: &pattie_ring_clock) -> Option<(u64, u64)> {
    let clock = (clock as *const pattie_ring_clock).cast::<pattie::abi::RingClock>();
    // SAFETY: a live, aligned clock pair with the same layout (abi_mirror.rs),
    // stored to by nobody else while this test reads it.
    unsafe { pattie::abi::read_clock(clock) }
}

#[test]
fn the_sdk_reads_back_the_pair_the_core_published() {
    let mut ring = Ring::new(4096);
    let never = sdk_reads(&ring.header().reader_clock);
    ring.app_write(&[0.5; 960]);
    ring.system_read(&mut [0.0; 480]);
    ring.system_publish_read_clock(777);

    let published = sdk_reads(&ring.header().reader_clock);

    assert_eq!((never, published), (None, Some((480, 777))));
}

#[test]
fn the_sdk_gives_up_on_a_pair_stuck_mid_write() {
    let mut ring = Ring::new(4096);
    ring.system_publish_read_clock(777);
    ring.app_header_mut().reader_clock.seq += 1; // odd: as if the driver died mid-write

    assert_eq!(sdk_reads(&ring.header().reader_clock), None);
}
