//! A Playout fed in another audio format.

use super::*;

#[test]
fn a_playout_from_another_format_converts_what_is_pushed() {
    let backend = FakeBackend::new().unwrap();
    let mic = Device::builder()
        .name("Agent Mic")
        .backend(backend.backend())
        .create()
        .unwrap();
    let pcm_24k = AudioFormat {
        rate: 24_000,
        channels: 1,
        encoding: crate::Encoding::I16,
    };
    let playout = Playout::manual(mic.inner().clone(), pcm_24k).unwrap();
    let bytes: Vec<u8> =
        (0..12_000i32) // half a second
            .flat_map(|i| i16::try_from(i % 1_000 * 20).unwrap().to_le_bytes())
            .collect();
    let mut expected = Vec::new();
    Converter::new(pcm_24k, AudioFormat::DEVICE)
        .unwrap()
        .convert(&bytes, &mut expected);

    // Pieces that split samples, as a model's audio deltas may.
    for piece in bytes.chunks(333) {
        playout.push_bytes(piece);
    }
    let queued: Vec<u8> = playout
        .shared
        .lock()
        .queue
        .iter()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();

    assert_eq!(playout.queued_frames(), 24_000);
    assert_eq!(queued, expected);
}
