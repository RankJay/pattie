//! The stable-ID hash exists once in the system, in the core (ADR 0015 amendment):
//! the driver and the Linux host take it from `pattie_core_device_info`. The SDK keeps
//! a copy in Rust (`pattie::stable_id`), because Windows apps compile no C (ADR
//! 0012). This checks the copy against the core, so the two can't drift apart.

use pattie::Kind;
use pattie_conformance::ffi::{PATTIE_KIND_MICROPHONE, PATTIE_KIND_SPEAKER};
use pattie_conformance::stable_id as core_stable_id;

/// The SDK's ID, back as the hash the core computes.
fn sdk_hash(kind: Kind, name: &str, rate: u32, channels: u16) -> u64 {
    let id = pattie::stable_id(kind, name, rate, channels);
    u64::from_str_radix(&id[1..], 16).expect("16 hex digits after the kind letter")
}

/// A tiny seeded generator (xorshift64), so a failure names a repeatable case.
struct Names(u64);

impl Names {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    /// A name of 1 to 63 UTF-16 units from a mix of ASCII, accented letters,
    /// CJK and emoji (surrogate pairs), as a Device name may hold.
    fn name(&mut self) -> String {
        const PICKS: [&str; 8] = ["a", "Z", " ", "é", "ß", "麦", "🎤", "-"];
        let mut name = String::new();
        let units = self.next() % 63 + 1;
        while name.encode_utf16().count() < units as usize {
            let pick = PICKS[(self.next() % PICKS.len() as u64) as usize];
            if name.encode_utf16().count() + pick.encode_utf16().count() > 63 {
                break;
            }
            name.push_str(pick);
        }
        if name.is_empty() {
            "a".to_string()
        } else {
            name
        }
    }
}

#[test]
fn the_sdks_stable_id_is_the_cores_for_every_name_and_format_tried() {
    let mut names = Names(0x9E37_79B9_7F4A_7C15);
    for _ in 0..5_000 {
        let name = names.name();
        // Every Device format, by the name's own randomness.
        let rate = [44_100, 48_000, 96_000][(names.next() % 3) as usize];
        let channels = (names.next() % 8 + 1) as u16;
        for (kind, core_kind) in [
            (Kind::Microphone, PATTIE_KIND_MICROPHONE),
            (Kind::Speaker, PATTIE_KIND_SPEAKER),
        ] {
            assert_eq!(
                sdk_hash(kind, &name, rate, channels),
                core_stable_id(core_kind, &name, rate, channels),
                "{kind:?} {name:?} {rate} {channels}"
            );
        }
    }
}

#[test]
fn the_kind_letter_is_the_one_each_host_writes() {
    assert!(pattie::stable_id(Kind::Microphone, "x", 48_000, 1).starts_with('M'));
    assert!(pattie::stable_id(Kind::Speaker, "x", 48_000, 1).starts_with('S'));
}
