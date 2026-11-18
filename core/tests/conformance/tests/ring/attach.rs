//! Attaching to a Ring: the memory, capacity and formats the Core accepts or refuses.

use super::*;

#[test]
fn memory_too_small_for_the_claimed_capacity_is_rejected() {
    let one_frame_short = Ring::bytes_for(256) - 4;

    let attached = Ring::attach(256, one_frame_short);

    assert_eq!(attached.err(), Some(PATTIE_ERR_BAD_RING));
}

#[test]
fn memory_too_small_for_the_header_is_rejected() {
    let attached = Ring::attach(256, 100);

    assert_eq!(attached.err(), Some(PATTIE_ERR_BAD_RING));
}

#[test]
fn capacity_outside_the_allowed_powers_of_two_is_rejected() {
    let bad = [0, 128, 300, 1000, 131_072];

    let results: Vec<_> = bad
        .iter()
        .map(|&capacity| Ring::attach(capacity, Ring::bytes_for(capacity)).err())
        .collect();

    assert_eq!(results, vec![Some(PATTIE_ERR_BAD_RING); bad.len()]);
}

#[test]
fn app_rewriting_capacity_after_attach_changes_nothing() {
    let mut ring = Ring::new(256);
    ring.app_write(&[0.0; 256]);
    ring.system_read(&mut [0.0; 256]); // next frames wrap to the buffer's start
    ring.app_write(&[0.5, 0.75]);
    ring.app_header_mut().capacity_frames = 1 << 30; // far beyond the real memory

    let mut out = [9.0f32; 2];
    ring.system_read(&mut out);

    assert_eq!(out, [0.5, 0.75]);
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
        (ring.system_sample_rate(), ring.system_channels()),
        (48_000, 1)
    );
}

#[test]
fn the_ring_reports_the_rate_and_channels_it_attached_with() {
    let reported: Vec<_> = [(44_100, 2), (96_000, 8), (48_000, 6)]
        .into_iter()
        .map(|(rate, channels)| {
            let ring = Ring::with_format(256, rate, channels);
            (ring.system_sample_rate(), ring.system_channels())
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

    assert_eq!(refused, [Some(PATTIE_ERR_BAD_RING); 5]);
}

#[test]
fn memory_sized_for_mono_is_too_small_for_stereo() {
    let attached = Ring::attach_format(256, 48_000, 2, Ring::bytes_for(256));

    assert_eq!(attached.err(), Some(PATTIE_ERR_BAD_RING));
}

#[test]
fn the_largest_ring_is_eight_channels_at_the_most_frames() {
    assert_eq!(
        PATTIE_RING_MAX_BYTES as usize,
        Ring::bytes_for_format(PATTIE_RING_MAX_FRAMES, PATTIE_RING_MAX_CHANNELS as u16)
    );
}

#[test]
fn app_rewriting_channels_after_attach_changes_nothing() {
    let mut ring = Ring::with_format(256, 48_000, 2);
    ring.app_header_mut().channels = 8; // the driver keeps its own copy

    ring.system_write(&stereo(0, 2));

    assert_eq!(ring.system_channels(), 2);
    assert_eq!(ring.system_counters().write, 2);
}
