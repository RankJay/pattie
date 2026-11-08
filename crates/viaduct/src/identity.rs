//! A Device's stable identity (ADR 0015): the same kind and name always give the
//! same ID, on every operating system, so the system treats a Device created again
//! as the same device and keeps the user's choice of it.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

use crate::Kind;

/// The stable identity of a Device with this kind and name.
///
/// It is `M` (Microphone) or `S` (Speaker), then the 64-bit FNV-1a hash of the
/// kind byte (1 or 2) followed by the name's UTF-16LE bytes, as 16 uppercase hex
/// digits; a hash of 0 becomes 1. Names are case-sensitive (ADR 0004), so
/// "Agent Mic" and "agent mic" differ.
///
/// Where it appears: on Windows, the Device's circuit device instance ID (the
/// driver writes it in lowercase; `platforms/windows/driver/src/endpoint.c`); on
/// Linux, the PipeWire `node.name` is `viaduct.` followed by it. Tools use it to
/// find a Device's endpoint or node.
///
/// ```
/// use viaduct::{Kind, stable_id};
///
/// assert_eq!(stable_id(Kind::Microphone, "Agent Mic"), "MBD5F5BFDA42F6F12");
/// ```
pub fn stable_id(kind: Kind, name: &str) -> String {
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
    format!("{letter}{:016X}", if hash == 0 { 1 } else { hash })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Vectors computed independently (a Python mirror of the spec), and confirmed
    /// identical to the Windows driver's own (a mirror of endpoint.c).
    #[test]
    fn stable_ids_match_the_drivers_vectors() {
        let ids = [
            stable_id(Kind::Microphone, "Agent Mic"),
            stable_id(Kind::Speaker, "Agent Speaker"),
            stable_id(Kind::Microphone, "agent mic"), // case-sensitive
            stable_id(Kind::Speaker, "Agent Mic"),    // kind is hashed first
        ];

        assert_eq!(
            ids,
            [
                "MBD5F5BFDA42F6F12",
                "SE4C7C9E6384B9C77",
                "MA22168868E50FF12",
                "S44E8BD5D8B4A93E3"
            ]
        );
    }
}
