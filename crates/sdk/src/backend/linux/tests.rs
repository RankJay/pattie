//! Why PipeWire is out of reach: Flatpak, PulseAudio, another user's session.

use super::*;

const ME: u32 = 1000;
const GREETER: u32 = 120; // gdm's login screen runs its own PulseAudio

fn processes(list: &[(u32, &str)]) -> Vec<(u32, String)> {
    list.iter()
        .map(|&(uid, comm)| (uid, comm.to_string()))
        .collect()
}

#[test]
fn this_users_pulseaudio_means_apps_cannot_see_pipewire_devices() {
    let running = processes(&[
        (
            ME,
            "pipewire
",
        ),
        (
            ME,
            "pulseaudio
",
        ),
    ]);

    assert!(pulseaudio_serves(running, ME));
}

#[test]
fn another_users_pulseaudio_is_not_this_users_sound_server() {
    let running = processes(&[
        (
            GREETER,
            "pulseaudio
",
        ),
        (
            ME,
            "pipewire
",
        ),
    ]);

    assert!(!pulseaudio_serves(running, ME));
}

#[test]
fn pipewires_own_pulseaudio_server_is_pipewire() {
    let running = processes(&[
        (
            ME,
            "pipewire
",
        ),
        (
            ME,
            "pipewire-pulse
",
        ),
    ]);

    assert!(!pulseaudio_serves(running, ME));
}

#[test]
fn pipewire_out_of_reach_in_a_flatpak_names_the_missing_permission() {
    assert_eq!(
        explain(Error::AudioServerUnavailable, true),
        Error::FlatpakNeedsPipeWire
    );
}

#[test]
fn outside_a_flatpak_pipewire_out_of_reach_stays_as_it_is() {
    assert_eq!(
        explain(Error::AudioServerUnavailable, false),
        Error::AudioServerUnavailable
    );
}

#[test]
fn other_errors_in_a_flatpak_stay_as_they_are() {
    assert_eq!(explain(Error::DuplicateName, true), Error::DuplicateName);
}

#[test]
fn the_process_list_includes_this_process_as_this_user() {
    let me = std::fs::read_to_string("/proc/self/comm").unwrap();
    let uid = this_user().unwrap();

    assert!(running_processes().any(|(owner, comm)| owner == uid && comm == me));
}
