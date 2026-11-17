//! A Device in PipeWire's graph: listed while it lives, back after a restart.

use super::*;

#[test]
#[ignore = "needs a running PipeWire (README, Linux)"]
fn a_device_is_listed_while_it_lives_and_gone_after() {
    let name = format!("Pattie Test Listed {}", std::process::id());
    let node = node_name(Kind::Microphone, &name);
    let listed = || {
        let dump = Command::new("pw-dump").output().unwrap().stdout;
        String::from_utf8_lossy(&dump).contains(&node)
    };

    let mic = Device::builder().name(&name).create().unwrap();
    let while_alive = listed();
    drop(mic);
    pause(300);

    assert!(while_alive, "{node} not listed while the Device lives");
    assert!(
        !listed(),
        "{node} still listed after the Device was dropped"
    );
}

#[test]
#[ignore = "restarts the user's PipeWire service (systemd --user); run on its own"]
fn a_device_comes_back_by_itself_after_pipewire_restarts() {
    let name = format!("Pattie Test Restart {}", std::process::id());
    let node = node_name(Kind::Microphone, &name);
    let listed = || {
        let dump = Command::new("pw-dump")
            .output()
            .map(|o| o.stdout)
            .unwrap_or_default();
        String::from_utf8_lossy(&dump).contains(&node)
    };
    let mic = Device::builder().name(&name).create().unwrap();
    let mut writer = mic.writer().unwrap();
    let file = std::env::temp_dir().join(format!("pattie-restart-{}.wav", std::process::id()));
    let stop = AtomicBool::new(false);

    let (back_after, samples) = std::thread::scope(|scope| {
        scope.spawn(|| {
            let (mut phase, mut chunk) = (0.0, [0.0f32; 480]);
            while !stop.load(Ordering::Relaxed) {
                tone(&mut chunk, &mut phase);
                let _ = writer.write(&chunk);
                pause(10);
            }
        });
        assert!(listed(), "not listed before the restart");
        let restarted = Command::new("systemctl")
            .args(["--user", "restart", "pipewire.service"])
            .status()
            .unwrap();
        assert!(restarted.success(), "systemctl: {restarted}");
        let since = Instant::now();
        while !listed() && since.elapsed() < Duration::from_secs(10) {
            pause(50);
        }
        let back_after = since.elapsed();
        let _ = Command::new("timeout")
            .args(["3", "pw-record", "--target", &node])
            .args(["--rate", "48000", "--channels", "1", "--format", "f32"])
            .arg(&file)
            .status();
        stop.store(true, Ordering::Relaxed);
        (
            back_after,
            wav_samples(&std::fs::read(&file).unwrap_or_default()),
        )
    });
    let _ = std::fs::remove_file(&file);

    println!("node back {back_after:?} after the restart command returned");
    // The brief's target: a usable device within 2 s of the audio service returning.
    assert!(
        back_after < Duration::from_secs(2),
        "back after {back_after:?}"
    );
    let steady = samples.get(24_000..).unwrap_or(&[]);
    assert!(
        steady.len() > 48_000,
        "recorded {} frames after the restart",
        samples.len()
    );
    let (rms, frequency) = measure(steady);
    assert!((rms - 0.354).abs() < 0.02, "rms {rms}");
    assert!((frequency - 440.0).abs() < 5.0, "frequency {frequency}");
}
