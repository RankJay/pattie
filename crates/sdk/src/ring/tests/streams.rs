//! An app and the driver using one Ring from two threads at once.

use super::*;
use std::thread;

/// Small enough to stay quick under Miri, which explores thread interleavings.
const STREAMED: u32 = 600;

#[test]
fn an_app_and_the_driver_stream_a_microphone_at_once() {
    let ring = RingMemory::new(64).unwrap();

    let heard = thread::scope(|scope| {
        scope.spawn(|| {
            let mut sent = 0;
            while sent < STREAMED {
                let chunk = ramp(sent, 17.min(STREAMED - sent));
                sent += u32::try_from(ring.write(&chunk)).unwrap();
                thread::yield_now();
            }
        });
        let driver = scope.spawn(|| {
            let mut heard = Vec::new();
            while heard.len() < STREAMED as usize {
                heard.extend(driver_take(&ring, 23.min(STREAMED as usize - heard.len())));
                thread::yield_now();
            }
            heard
        });
        driver.join().unwrap()
    });

    assert_eq!(heard, ramp(0, STREAMED));
}

#[test]
fn the_driver_and_an_app_stream_a_speaker_at_once() {
    let ring = RingMemory::new(64).unwrap();

    let heard = thread::scope(|scope| {
        scope.spawn(|| {
            let mut played = 0;
            while played < STREAMED {
                let chunk = ramp(played, 19.min(STREAMED - played));
                played += u32::try_from(driver_play(&ring, &chunk)).unwrap();
                thread::yield_now();
            }
        });
        let app = scope.spawn(|| {
            let mut heard = Vec::new();
            let mut out = [0.0; 13];
            while heard.len() < STREAMED as usize {
                let count = ring.read(&mut out);
                heard.extend_from_slice(&out[..count]);
                thread::yield_now();
            }
            heard
        });
        app.join().unwrap()
    });

    assert_eq!(heard, ramp(0, STREAMED));
}
