//! The Ring on its own, with the system's side played in Rust by the same counter rules.
use super::*;
use std::sync::atomic::Ordering;

mod streams;

/// The driver taking up to `wanted` frames from a Microphone Ring; what it
/// asks for and doesn't get is counted as Gap.
fn driver_take(ring: &RingMemory, wanted: usize) -> Vec<f32> {
    let written = ring.write_frames().load(Ordering::Acquire);
    let start = ring.read_frames().load(Ordering::Relaxed); // only the driver stores it
    let count = wanted.min(usize::try_from(written - start).unwrap());
    let taken: Vec<f32> = (0..count)
        // SAFETY: below the write counter just acquired, so published.
        .map(|i| unsafe { ring.frame(ring.slot(start + i as u64)).read() })
        .collect();
    let gap = ring.gap_frames();
    gap.store(
        gap.load(Ordering::Relaxed) + (wanted - count) as u64,
        Ordering::Release,
    );
    ring.read_frames()
        .store(start + count as u64, Ordering::Release);
    taken
}

/// The driver playing `samples` into a Speaker Ring, as many as fit.
fn driver_play(ring: &RingMemory, samples: &[f32]) -> usize {
    let read = ring.read_frames().load(Ordering::Acquire);
    let start = ring.write_frames().load(Ordering::Relaxed); // only the driver stores it
    let room = usize::try_from(ring.capacity() - (start - read)).unwrap();
    let count = samples.len().min(room);
    for (i, &sample) in samples[..count].iter().enumerate() {
        // SAFETY: at or past the write counter and within the room the
        // acquired read counter allows, so the app isn't reading it.
        unsafe { ring.frame(ring.slot(start + i as u64)).write(sample) };
    }
    ring.write_frames()
        .store(start + count as u64, Ordering::Release);
    count
}

fn ramp(from: u32, count: u32) -> Vec<f32> {
    (from..from + count).map(|n| n as f32).collect()
}

#[test]
fn a_new_ring_is_aligned_zeroed_and_knows_its_capacity() {
    let ring = RingMemory::new(64).unwrap();

    assert_eq!(ring.address() % 64, 0);
    assert_eq!(ring.len(), size_of::<RingHeader>() + 64 * size_of::<f32>());
    assert_eq!(ring.capacity(), 64);
    assert_eq!((ring.buffered(), ring.counts()), (0, (0, 0)));
    // SAFETY: frame 63 is the last one inside the allocation.
    assert_eq!(unsafe { ring.frame(63).read() }, 0.0);
}

#[test]
fn microphone_audio_wraps_around_the_end_in_order() {
    let ring = RingMemory::new(64).unwrap();
    assert_eq!(ring.write(&ramp(0, 50)), 50);
    assert_eq!(driver_take(&ring, 40), ramp(0, 40));

    let accepted = ring.write(&ramp(50, 50)); // slots 50..63, then 0..35

    assert_eq!(accepted, 50);
    assert_eq!(driver_take(&ring, 60), ramp(40, 60));
}

#[test]
fn a_full_microphone_ring_drops_and_counts_the_rest() {
    let ring = RingMemory::new(64).unwrap();

    let accepted = (ring.write(&ramp(0, 70)), ring.write(&[1.0; 5]));

    assert_eq!(accepted, (64, 0));
    assert_eq!(ring.counts(), (0, 11));
    assert_eq!(driver_take(&ring, 64), ramp(0, 64)); // nothing overwritten
}

#[test]
fn what_the_driver_asks_for_and_doesnt_get_is_gap() {
    let ring = RingMemory::new(64).unwrap();
    ring.write(&ramp(0, 10));

    let taken = driver_take(&ring, 25);

    assert_eq!((taken, ring.counts()), (ramp(0, 10), (15, 0)));
}

#[test]
fn speaker_audio_wraps_around_the_end_in_order() {
    let ring = RingMemory::new(64).unwrap();
    assert_eq!(driver_play(&ring, &ramp(0, 60)), 60);
    let mut out = [0.0; 100];
    assert_eq!(ring.read(&mut out[..30]), 30);

    assert_eq!(driver_play(&ring, &ramp(60, 30)), 30); // slots 60..63, 0..25
    let count = ring.read(&mut out);

    assert_eq!(&out[..count], ramp(30, 60).as_slice());
}

#[test]
fn buffered_never_exceeds_the_capacity_whatever_the_counters_claim() {
    let ring = RingMemory::new(64).unwrap();
    ring.write_frames().store(1_000, Ordering::Release);

    let ahead = ring.buffered();
    ring.read_frames().store(2_000, Ordering::Release);

    assert_eq!((ahead, ring.buffered()), (64, 0));
}

#[test]
fn a_hostile_read_counter_never_lets_the_app_write_outside_the_ring() {
    let ring = RingMemory::new(64).unwrap();
    ring.read_frames().store(u64::MAX, Ordering::Release); // "read" far ahead

    let accepted = ring.write(&ramp(0, 100));

    // Counted as nothing unread: one full Ring accepted, the rest dropped.
    assert_eq!((accepted, ring.counts()), (64, (0, 36)));
}

#[test]
fn a_hostile_write_counter_never_lets_the_app_read_outside_the_ring() {
    let ring = RingMemory::new(64).unwrap();
    ring.write_frames().store(u64::MAX, Ordering::Release);

    let mut out = [9.0; 100];
    let count = ring.read(&mut out);

    assert_eq!(count, 64);
}
