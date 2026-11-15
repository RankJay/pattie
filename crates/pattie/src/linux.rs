//! The Linux backend (ADR 0018): the app's own process plays the driver. The Core
//! host (`crate::core_host`) runs the embedded C core, and each Device becomes a
//! PipeWire node that streams through its Ring ([`pipewire`]).

pub(crate) mod pipewire;

use crate::Error;
use crate::core_host::HostTransport;
use std::os::unix::fs::MetadataExt;
use std::sync::Arc;

/// PipeWire's graph pulls a whole cycle at once: 1,024 frames (21.3 ms) by
/// default, up to 2,048 at its default maximum. Leading by 960 left every
/// 1,024-frame pull 64 frames short (WSL, 2026-11-08).
const LEAD_FRAMES: u32 = 2_048;

/// The default transport on Linux: the process's core and real PipeWire nodes.
/// `PulseAudioInUse` where PulseAudio is this user's sound server: its apps would
/// never see a PipeWire node, so a Device created there would be invisible.
pub(crate) fn transport() -> Result<HostTransport, Error> {
    if this_user().is_some_and(|uid| pulseaudio_serves(running_processes(), uid)) {
        return Err(Error::PulseAudioInUse);
    }
    HostTransport::for_process(Arc::new(pipewire::PipeWireNodes), LEAD_FRAMES)
}

/// What a failure to reach PipeWire means here: in a Flatpak sandbox
/// (`sandboxed`), the one permission the app lacks; anything else, as it is.
fn explain(error: Error, sandboxed: bool) -> Error {
    match error {
        Error::AudioServerUnavailable if sandboxed => Error::FlatpakNeedsPipeWire,
        other => other,
    }
}

/// Whether this process runs in a Flatpak sandbox: Flatpak puts this file at
/// the root of every sandbox it runs.
fn in_flatpak() -> bool {
    std::path::Path::new("/.flatpak-info").exists()
}

/// Whether a `pulseaudio` daemon runs as `uid`: then it is that user's sound
/// server, whether or not PipeWire runs too (Ubuntu 22.04 runs PipeWire for
/// screen sharing and PulseAudio for audio). PipeWire's own PulseAudio server is
/// `pipewire-pulse`, and another user's daemon (the login screen's) serves only
/// that user. `processes` are (owner, name) pairs, names as /proc gives them.
fn pulseaudio_serves(processes: impl IntoIterator<Item = (u32, String)>, uid: u32) -> bool {
    processes
        .into_iter()
        .any(|(owner, name)| owner == uid && name.trim_end() == "pulseaudio")
}

/// This process's user ID, from /proc (no call into libc).
fn this_user() -> Option<u32> {
    std::fs::metadata("/proc/self").ok().map(|meta| meta.uid())
}

/// Every process this one can see, as (owner, name). Processes that end while
/// being read are skipped.
fn running_processes() -> impl Iterator<Item = (u32, String)> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .bytes()
                .all(|b| b.is_ascii_digit())
        })
        .filter_map(|entry| {
            let owner = entry.metadata().ok()?.uid();
            let name = std::fs::read_to_string(entry.path().join("comm")).ok()?;
            Some((owner, name))
        })
}

#[cfg(test)]
mod tests {
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
}
