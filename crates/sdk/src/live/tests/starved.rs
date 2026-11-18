//! Silence while the source is short of audio, and how it is counted.

use super::*;

#[test]
fn it_plays_silence_until_the_buffer_holds_its_target_then_the_audio() {
    let (backend, mic, live, lead) = setup_with_lead();
    live.push(&[0.5; 960]);
    live.shared.pump();
    let filling = backend.system_read(mic.id(), lead).unwrap(); // the Ring's lead

    live.push(&[0.5; 1_920]);
    live.shared.pump(); // the queue holds its target: audio
    let playing = backend.system_read(mic.id(), 480).unwrap();

    assert!(filling.iter().all(|&s| s == 0.0));
    assert!(
        playing.iter().all(|&s| (s - 0.5).abs() < 1e-3),
        "{:?}",
        &playing[..4]
    );
    assert_eq!(live.stats().starved_frames, 0);
}

#[test]
fn the_starved_count_is_the_silence_the_app_hears() {
    let (backend, mic, live, lead) = setup_with_lead();
    live.push(&vec![0.5; target(&live)]);
    live.shared.pump();
    // The system pulls 10 ms at a time and nothing more arrives, until the
    // queue runs short in the middle of a pull.
    while live.stats().starved_frames == 0 {
        backend.system_read(mic.id(), 480).unwrap();
        live.shared.pump();
    }

    let heard = backend.system_read(mic.id(), lead).unwrap();
    let silent_tail = heard.iter().rev().take_while(|&&s| s == 0.0).count();

    assert!(
        heard[0] != 0.0,
        "the Ring starts with the last of the audio"
    );
    assert_eq!(live.stats().starved_frames, silent_tail as u64);
}

#[test]
fn a_stereo_starved_count_is_the_silent_frames_the_app_hears() {
    let (backend, mic, live, lead) = setup_format(48_000, 2);
    live.push(&vec![0.5; target(&live) * 2]);
    live.shared.pump();
    for _ in 0..20 {
        backend.system_read(mic.id(), 480).unwrap();
        live.shared.pump();
        if live.stats().starved_frames > 0 {
            break;
        }
    }

    let heard = backend.system_read(mic.id(), lead).unwrap();
    let silent_tail = heard
        .chunks_exact(2)
        .rev()
        .take_while(|frame| frame.iter().all(|&s| s == 0.0))
        .count();

    assert!(silent_tail > 0, "the source fell behind within 20 passes");
    assert_eq!(live.stats().starved_frames, silent_tail as u64);
}

#[test]
fn a_source_that_falls_behind_plays_counted_silence_then_fills_up_again() {
    let (backend, mic, live) = setup();
    live.push(&vec![0.5; target(&live)]);
    live.shared.pump();
    for _ in 0..6 {
        // The system pulls 10 ms at a time; nothing more arrives.
        backend.system_read(mic.id(), 480).unwrap();
        live.shared.pump();
    }
    let starved = live.stats().starved_frames;

    live.push(&[0.5; 960]); // under the target: still silence
    backend.system_read(mic.id(), 960).unwrap();
    live.shared.pump();
    let refilling = backend.system_read(mic.id(), 480).unwrap();

    assert!(starved > 0);
    assert!(refilling.iter().all(|&s| s == 0.0));
    assert_eq!(live.stats().queued_frames, 960);
}
