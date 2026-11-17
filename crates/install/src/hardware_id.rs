//! The driver's hardware IDs, and the multi-string form SetupAPI stores them in.

use super::utf16le_units;

/// The root device the driver serves (pattie.inf, ADR 0010).
pub const ROOT_HARDWARE_ID: &str = r"Root\Pattie";
/// Each Device's runtime child (ADR 0013).
pub const ENDPOINT_HARDWARE_ID: &str = r"Pattie\Endpoint";

/// Splits a REG_MULTI_SZ value (UTF-16LE bytes) into its strings. An empty string
/// ends the list, as in the registry.
pub fn parse_multi_sz(bytes: &[u8]) -> Vec<String> {
    utf16le_units(bytes)
        .split(|&unit| unit == 0)
        .take_while(|part| !part.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

/// Encodes strings as a REG_MULTI_SZ value (UTF-16LE, each NUL-terminated, plus a
/// final NUL).
pub fn to_multi_sz(strings: &[&str]) -> Vec<u8> {
    let mut units: Vec<u16> = Vec::new();
    for string in strings {
        units.extend(string.encode_utf16());
        units.push(0);
    }
    units.push(0);
    units.iter().flat_map(|unit| unit.to_le_bytes()).collect()
}

/// Whether `ids` holds `wanted`. PnP compares hardware IDs without case.
pub fn has_hardware_id(ids: &[String], wanted: &str) -> bool {
    ids.iter().any(|id| id.eq_ignore_ascii_case(wanted))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inf_lists_hardware_id;

    #[test]
    fn root_and_endpoint_ids_match_the_inf() {
        let inf = include_str!("../../../driver/pattie.inf");
        assert!(inf_lists_hardware_id(inf, ROOT_HARDWARE_ID));
        assert!(inf_lists_hardware_id(inf, ENDPOINT_HARDWARE_ID));
        assert_ne!(ROOT_HARDWARE_ID, ENDPOINT_HARDWARE_ID);
    }

    #[test]
    fn multi_sz_round_trips() {
        let bytes = to_multi_sz(&[r"Root\Pattie", "Second"]);
        assert_eq!(bytes.len(), (11 + 1 + 6 + 1 + 1) * 2); // "Root\Pattie", "Second", end
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
        assert_eq!(parse_multi_sz(&bytes), vec![r"Root\Pattie", "Second"]);
    }

    #[test]
    fn multi_sz_stops_at_the_empty_string_and_ignores_a_trailing_odd_byte() {
        let mut bytes = to_multi_sz(&["a"]);
        bytes.extend(to_multi_sz(&["ignored"]));
        bytes.push(0x41);
        assert_eq!(parse_multi_sz(&bytes), vec!["a"]);
        assert!(parse_multi_sz(&[]).is_empty());
    }

    #[test]
    fn hardware_ids_compare_without_case() {
        let ids = vec![r"ROOT\PATTIE".to_string(), "Other".to_string()];
        assert!(has_hardware_id(&ids, r"Root\Pattie"));
        assert!(!has_hardware_id(&ids, r"Root\Pattie2"));
        assert!(!has_hardware_id(&[], r"Root\Pattie"));
    }
}
