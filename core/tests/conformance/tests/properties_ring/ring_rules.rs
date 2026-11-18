//! The property tests for one Ring: a Microphone (checked against `MicModel`), a Speaker,
//! and an app that scribbles on the header.

use super::*;

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn a_microphone_delivers_exactly_what_the_model_says(
        capacity in capacity(),
        ops in prop::collection::vec(mic_op(), 1..60),
    ) {
        let mut ring = Ring::new(capacity);
        let mut model = MicModel { capacity: u64::from(capacity), written: 0, read: 0, gap: 0 };

        for op in ops {
            match op {
                MicOp::AppWrite(n) => {
                    let samples: Vec<f32> = (model.written..model.written + n as u64).map(sample)
                        .collect();
                    ring.app_write(&samples);
                    model.written += n as u64;
                }
                MicOp::DriverRead(n) => {
                    let mut out = vec![-1.0f32; n];
                    ring.system_read(&mut out);
                    let expected: Vec<u32> = model.read(n).iter()
                        .map(|frame| frame.map_or(0, |i| sample(i).to_bits())).collect();
                    let got: Vec<u32> = out.iter().map(|s| s.to_bits()).collect();
                    prop_assert_eq!(got, expected);
                }
                MicOp::DriverReadPcm16(n) => {
                    let mut out = vec![-1i16; n];
                    ring.system_read_pcm16(&mut out);
                    let expected: Vec<i16> = model.read(n).iter()
                        .map(|frame| frame.map_or(0, |i| to_pcm16(sample(i).to_bits()))).collect();
                    prop_assert_eq!(out, expected);
                }
                MicOp::SkipStale(keep) => {
                    ring.system_skip_stale(keep);
                    model.skip_stale(keep);
                }
            }
            let counters = ring.system_counters();
            prop_assert_eq!((counters.read, counters.gap), (model.read, model.gap));
            prop_assert_eq!(ring.header().gap_frames, model.gap, "Gap published to the app");
            prop_assert!(ring.guard_intact(), "the driver wrote past the Ring");
        }
    }

    #[test]
    fn a_speaker_hands_the_app_exactly_what_the_driver_kept(
        capacity in capacity(),
        ops in prop::collection::vec(speaker_op(), 1..60),
    ) {
        let mut ring = Ring::new(capacity);
        let mut queue: VecDeque<u32> = VecDeque::new(); // unread frames, as bits
        let (mut offered, mut overflow) = (0u64, 0u64);

        for op in ops {
            match op {
                SpeakerOp::DriverWrite(n) | SpeakerOp::DriverWritePcm16(n) => {
                    let room = capacity as usize - queue.len();
                    let kept = n.min(room);
                    let pcm = matches!(op, SpeakerOp::DriverWritePcm16(_));
                    if pcm {
                        let src: Vec<i16> = (0..n as u64).map(|i| ((offered + i) % 30_000) as i16)
                            .collect();
                        ring.system_write_pcm16(&src);
                        queue.extend(src[..kept].iter().map(|&p| from_pcm16(p)));
                    } else {
                        let src: Vec<f32> = (offered..offered + n as u64).map(sample).collect();
                        ring.system_write(&src);
                        queue.extend(src[..kept].iter().map(|s| s.to_bits()));
                    }
                    offered += n as u64;
                    overflow += (n - kept) as u64; // a full Ring drops the newest frames
                }
                SpeakerOp::AppRead(n) => {
                    let got: Vec<u32> = ring.app_read(n).iter().map(|s| s.to_bits()).collect();
                    let expected: Vec<u32> = queue.drain(..n.min(queue.len())).collect();
                    prop_assert_eq!(got, expected);
                }
            }
            prop_assert_eq!(ring.system_counters().overflow, overflow);
            prop_assert_eq!(ring.header().overflow_frames, overflow,
                "Overflow published to the app");
            prop_assert!(ring.guard_intact(), "the driver wrote past the Ring");
        }
    }

    #[test]
    fn a_ring_the_app_scribbles_on_never_leads_the_driver_astray(
        capacity in capacity(),
        ops in prop::collection::vec(hostile_op(), 1..80),
    ) {
        let mut ring = Ring::new(capacity);

        for op in ops {
            let before = ring.system_counters();
            let (shared_write, shared_read) = (ring.header().write_frames, ring
                .header().read_frames);
            let cap = u64::from(capacity);
            match op {
                HostileOp::SetWrite(v) => ring.app_header_mut().write_frames = v,
                HostileOp::SetRead(v) => ring.app_header_mut().read_frames = v,
                HostileOp::SetWriteNearRead(d) => {
                    ring.app_header_mut().write_frames = before.read
                        .wrapping_add_signed(i64::from(d));
                }
                HostileOp::SetReadNearWrite(d) => {
                    ring.app_header_mut().read_frames = before.write
                        .wrapping_add_signed(i64::from(d));
                }
                HostileOp::SetCapacity(v) => ring.app_header_mut().capacity_frames = v,
                HostileOp::SetGap(v) => ring.app_header_mut().gap_frames = v,
                HostileOp::SetOverflow(v) => ring.app_header_mut().overflow_frames = v,
                HostileOp::DriverRead(n) | HostileOp::DriverReadPcm16(n) => {
                    if matches!(op, HostileOp::DriverRead(_)) {
                        ring.system_read(&mut vec![0.0f32; n]);
                    } else {
                        ring.system_read_pcm16(&mut vec![0i16; n]);
                    }
                    // What the header allows: nothing if the writer moved backwards;
                    // at most a Ring's worth (an overrun skips to the oldest intact frame).
                    let available = shared_write.checked_sub(before.read).map_or(0, |a| a.min(cap));
                    let taken = available.min(n as u64);
                    let skipped = shared_write.checked_sub(before.read).map_or(0, |a| a
                        .saturating_sub(cap));
                    let after = ring.system_counters();
                    prop_assert_eq!(after.gap - before.gap, n as u64 - taken,
                        "every frame delivered or a Gap");
                    prop_assert_eq!(after.read - before.read, skipped + taken);
                    prop_assert_eq!(ring.header().read_frames, after.read);
                    prop_assert_eq!(ring.header().gap_frames, after.gap);
                }
                HostileOp::DriverWrite(n) | HostileOp::DriverWritePcm16(n) => {
                    if matches!(op, HostileOp::DriverWrite(_)) {
                        ring.system_write(&vec![0.5f32; n]);
                    } else {
                        ring.system_write_pcm16(&vec![1000i16; n]);
                    }
                    // Room the header allows: a reader counter ahead of the writer is
                    // impossible and ignored; one a Ring or more behind means full.
                    let unread = before.write.checked_sub(shared_read).map_or(0, |u| u.min(cap));
                    let stored_expected = (cap - unread).min(n as u64);
                    let after = ring.system_counters();
                    let (stored, dropped) = (after.write - before.write,
                        after.overflow - before.overflow);
                    prop_assert_eq!(stored, stored_expected, "only the room there is");
                    prop_assert_eq!(dropped, n as u64 - stored, "the rest counted as Overflow");
                    prop_assert_eq!(ring.header().write_frames, after.write);
                    prop_assert_eq!(ring.header().overflow_frames, after.overflow);
                }
                HostileOp::SkipStale(keep) => {
                    ring.system_skip_stale(keep);
                    prop_assert_eq!(ring.system_counters().gap, before.gap,
                        "skipping is not a Gap");
                }
            }
            let after = ring.system_counters();
            prop_assert_eq!(after.capacity, capacity, "the capacity is fixed at attach");
            prop_assert!(after.read >= before.read && after.write >= before.write,
                "counters only move forward");
            prop_assert!(ring.guard_intact(), "the driver wrote past the Ring");
        }
    }
}
