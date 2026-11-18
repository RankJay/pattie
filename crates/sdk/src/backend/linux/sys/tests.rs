//! The stream's format POD for each channel layout, and the stream's flags.
use super::*;

#[test]
fn the_mono_48_khz_pod_is_word_for_word_the_one_v0_offered() {
    // v0's static pod, which PipeWire accepted (ADR 0018).
    let v0: [u32; 42] = [
        160, 15, 0x4_0003, 3, 1, 0, 4, 3, 1, 0, 2, 0, 4, 3, 1, 0, 0x1_0001, 0, 4, 3, 0x11b, 0,
        0x1_0003, 0, 4, 4, 48_000, 0, 0x1_0004, 0, 4, 4, 1, 0, 0x1_0005, 0, 12, 13, 4, 3, 2, 0,
    ];

    assert_eq!(format_pod(48_000, 1).words(), v0);
}

#[test]
fn a_stereo_44_1_khz_pod_gives_its_rate_channels_and_front_left_and_right() {
    let pod = format_pod(44_100, 2);
    let words = pod.words();

    assert_eq!(words[22..28], [0x1_0003, 0, 4, 4, 44_100, 0]); // rate
    assert_eq!(words[28..34], [0x1_0004, 0, 4, 4, 2, 0]); // channels
    // Array of 2 Ids: 8 bytes of child header and 8 of FL, FR; no padding.
    assert_eq!(words[34..], [0x1_0005, 0, 16, 13, 4, 3, 3, 4]);
}

#[test]
fn every_layout_is_one_object_sized_by_its_header_and_padded_to_8_bytes() {
    for channels in 1..=8u16 {
        let pod = format_pod(96_000, channels);
        let words = pod.words();
        let bytes = words.len() * 4;

        // The size field counts the body after the 8-byte (size, type) header.
        assert_eq!(words[0] as usize, bytes - 8, "{channels}");
        assert_eq!(words[1], 15, "{channels}"); // SPA_TYPE_Object
        assert_eq!(bytes % 8, 0, "{channels}");
        assert_eq!(
            words[36] as usize,
            8 + 4 * usize::from(channels),
            "{channels}"
        );
        assert_eq!(positions(&pod).len(), usize::from(channels));
    }
    assert_eq!(std::mem::align_of::<Pod>(), 8);
}

/// The channel positions a pod gives.
fn positions(pod: &Pod) -> &[u32] {
    let words = pod.words();
    let count = (words[36] as usize - 8) / 4;
    &words[40..40 + count]
}

#[test]
fn the_layouts_are_the_standard_ones_windows_also_offers() {
    // spa/param/audio/raw.h: MONO 2, FL 3, FR 4, FC 5, LFE 6, SL 7, SR 8,
    // RC 11, RL 12, RR 13. Windows' masks, in its order: 3.0, quad, 5.0,
    // 5.1, 6.1, 7.1 surround.
    let layouts: Vec<Vec<u32>> = (1..=8)
        .map(|channels| positions(&format_pod(48_000, channels)).to_vec())
        .collect();

    assert_eq!(
        layouts,
        [
            vec![2],
            vec![3, 4],
            vec![3, 4, 5],
            vec![3, 4, 12, 13],
            vec![3, 4, 5, 12, 13],
            vec![3, 4, 5, 6, 12, 13],
            vec![3, 4, 5, 6, 12, 13, 11],
            vec![3, 4, 5, 6, 12, 13, 7, 8],
        ]
    );
}

#[test]
fn the_stream_is_never_auto_connected() {
    const AUTOCONNECT: c_int = 1 << 0;

    assert_eq!(STREAM_FLAGS & AUTOCONNECT, 0);
}
