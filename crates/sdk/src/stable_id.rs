//! A Device's stable identity (ADR 0015): the same kind, name and format always
//! give the same ID, on every operating system, so the system treats a Device
//! created again as the same device and keeps the user's choice of it.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use crate::Kind;

/// The stable identity of a Device with this kind, name and format (its sample
/// rate and channels, as given to `DeviceBuilder::format`; 48,000 and 1 by
/// default).
///
/// It is `M` (Microphone) or `S` (Speaker), then the 64-bit FNV-1a hash of the
/// kind byte (1 or 2), the name's UTF-16LE bytes, the rate's 4 bytes and the
/// channel count's 2 (little-endian), as 16 uppercase hex digits; a hash of 0
/// becomes 1. Names are case-sensitive (ADR 0004), so "Agent Mic" and "agent mic"
/// differ. The same name in another format is another ID: Windows keeps a format
/// per endpoint, and would otherwise open the Device in one it no longer offers.
///
/// The core computes the same hash (`pattie_stable_id`), and the system uses the
/// core's; this copy is for apps and tools, and is tested against it.
///
/// Where it appears: on Windows, the Device's circuit device instance ID (the
/// driver writes it in lowercase; `driver/src/endpoint.c`); on
/// Linux, the PipeWire `node.name` is `pattie.` followed by it. Tools use it to
/// find a Device's endpoint or node.
///
/// ```
/// use pattie::{Kind, stable_id};
///
/// assert_eq!(stable_id(Kind::Microphone, "Agent Mic", 48_000, 1), "M41B28DFC5B21A0BE");
/// ```
pub fn stable_id(kind: Kind, name: &str, sample_rate: u32, channels: u16) -> String {
    let (letter, kind_byte) = match kind {
        Kind::Microphone => ('M', 1u8),
        Kind::Speaker => ('S', 2u8),
    };
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut feed = |byte: u8| {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    };
    feed(kind_byte);
    for unit in name.encode_utf16() {
        for byte in unit.to_le_bytes() {
            feed(byte);
        }
    }
    for byte in sample_rate
        .to_le_bytes()
        .into_iter()
        .chain(channels.to_le_bytes())
    {
        feed(byte);
    }
    format!("{letter}{:016X}", if hash == 0 { 1 } else { hash })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors computed independently (a Python mirror of the spec, 2026-11-14);
    /// the core's own hash is checked against this function in core/tests.
    #[test]
    fn stable_ids_match_the_drivers_vectors() {
        let ids = [
            stable_id(Kind::Microphone, "Agent Mic", 48_000, 1),
            stable_id(Kind::Speaker, "Agent Speaker", 48_000, 1),
            stable_id(Kind::Microphone, "agent mic", 48_000, 1), // case-sensitive
            stable_id(Kind::Speaker, "Agent Mic", 48_000, 1),    // kind is hashed first
            stable_id(Kind::Microphone, "Agent Mic", 96_000, 1), // and the format last
            stable_id(Kind::Microphone, "Agent Mic", 48_000, 2),
        ];

        assert_eq!(
            ids,
            [
                "M41B28DFC5B21A0BE",
                "S29395CD2B4F87993",
                "M0B0CD97701FAB0BE",
                "S9D5451AAD4749C6F",
                "M92A123DFD87AF89B",
                "M41AF27FC5B1EBD95"
            ]
        );
    }
}
