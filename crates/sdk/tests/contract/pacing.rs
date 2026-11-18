//! Paced writes and reads, and waiting for playout.

use super::*;

contract_suite!(
    write_paced_delivers_everything_keeping_only_a_small_lead,
    write_paced_stops_at_its_deadline_with_only_the_lead_written,
    wait_for_playout_is_true_once_the_system_has_taken_everything,
    wait_for_playout_waits_while_the_system_takes_the_audio,
    read_paced_returns_as_soon_as_the_system_plays_audio,
    read_paced_returns_nothing_once_its_deadline_passes,
    read_paced_into_room_for_one_frame_waits_for_it,
);

fn write_paced_delivers_everything_keeping_only_a_small_lead(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let samples: Vec<f32> = (0..9_600u16).map(|i| f32::from(i) / 9_600.0).collect(); // 200 ms
    let (written, heard, most_buffered) = std::thread::scope(|scope| {
        let system = scope.spawn(|| {
            // Takes only what's there, so the order of real frames can be checked.
            let (mut heard, mut most) = (Vec::new(), 0);
            let give_up = Instant::now() + Duration::from_secs(5);
            while heard.len() < samples.len() && Instant::now() < give_up {
                let buffered = mic.stats().buffered_frames;
                most = most.max(buffered);
                let take = usize::try_from(buffered.min(480)).unwrap();
                if take > 0 {
                    heard.extend(backend.system_read(mic.id(), take).unwrap());
                }
                system_pause();
            }
            (heard, most)
        });
        let deadline = Instant::now() + Duration::from_secs(5);
        let written = tx.write_paced(&samples, deadline).unwrap();
        let (heard, most) = system.join().unwrap();
        (written, heard, most)
    });

    assert_eq!(written, samples.len());
    assert_eq!(heard, samples);
    assert!(
        most_buffered <= 2_400,
        "{most_buffered} frames buffered, over 50 ms"
    );
    assert_eq!(mic.stats().overflow_frames, 0);
}

fn write_paced_stops_at_its_deadline_with_only_the_lead_written(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    let started = Instant::now();

    // Nobody pulls: the lead fills, then nothing more fits before the deadline.
    let written = tx
        .write_paced(&[0.5; 48_000], started + Duration::from_millis(50))
        .unwrap();

    assert!((1..=2_400).contains(&written), "{written} written");
    assert!(started.elapsed() < Duration::from_secs(1));
    assert_eq!(mic.stats().overflow_frames, 0);
}

fn wait_for_playout_is_true_once_the_system_has_taken_everything(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 960]).unwrap();

    let while_queued = tx.wait_for_playout(Instant::now() + Duration::from_millis(30));
    backend.system_read(mic.id(), 960);
    let once_taken = tx.wait_for_playout(Instant::now() + Duration::from_secs(1));

    assert_eq!((while_queued, once_taken), (false, true));
}

fn wait_for_playout_waits_while_the_system_takes_the_audio(backend: &dyn BackendUnderTest) {
    let mic = builder(backend).name("Agent Mic").create().unwrap();
    let mut tx = mic.writer().unwrap();
    tx.write(&[0.5; 960]).unwrap();
    let started = Instant::now();

    let played = std::thread::scope(|scope| {
        scope.spawn(|| {
            sleep_ms(50);
            backend.system_read(mic.id(), 960);
        });
        tx.wait_for_playout(started + Duration::from_secs(10))
    });

    assert!(played);
    assert!(started.elapsed() < Duration::from_secs(5));
}

fn read_paced_returns_as_soon_as_the_system_plays_audio(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let mut out = [0.0f32; 960];
    let started = Instant::now();

    // A long deadline: only the doorbell the system rings can end the wait early.
    let got = std::thread::scope(|scope| {
        scope.spawn(|| {
            sleep_ms(50);
            backend.system_write(speaker.id(), &[0.25; 480]);
        });
        rx.read_paced(&mut out, started + Duration::from_secs(10))
            .unwrap()
    });

    assert_eq!(got, 480);
    assert!(out[..480].iter().all(|&s| s == 0.25));
    assert!(started.elapsed() < Duration::from_secs(5));
}

fn read_paced_returns_nothing_once_its_deadline_passes(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let started = Instant::now();

    let got = rx
        .read_paced(&mut [0.0; 480], started + Duration::from_millis(30))
        .unwrap();

    assert_eq!(got, 0);
    assert!(started.elapsed() >= Duration::from_millis(14)); // a timer tick early at most
}

fn read_paced_into_room_for_one_frame_waits_for_it(backend: &dyn BackendUnderTest) {
    let speaker = builder(backend)
        .name("Agent Speaker")
        .kind(Kind::Speaker)
        .create()
        .unwrap();
    let mut rx = speaker.reader().unwrap();
    let mut out = [0.0f32; 1];
    let started = Instant::now();

    let got = std::thread::scope(|scope| {
        scope.spawn(|| {
            sleep_ms(50);
            backend.system_write(speaker.id(), &[0.25; 480]);
        });
        rx.read_paced(&mut out, started + Duration::from_secs(10))
            .unwrap()
    });

    assert_eq!((got, out[0]), (1, 0.25));
}
