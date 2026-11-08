//! `viaduct-cli install <package>` and `viaduct-cli uninstall`: put the driver on
//! this machine, or take it off (ADR 0016). Both need an administrator PowerShell.
//!
//! The parsing helpers are plain functions, tested on every platform; the SetupAPI
//! calls that change the machine are Windows-only and are checked in the test VM.

// Outside tests only the Windows installer (install/setup.rs) calls the helpers.
// `allow`, not `expect`: the tests use them on every OS, so the lint fires only in
// a non-test build elsewhere, and `expect` would fail the test build.
#![cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "only the Windows installer uses these outside tests"
    )
)]

/// The root device the driver serves (viaduct.inf, ADR 0010).
pub const ROOT_HARDWARE_ID: &str = r"Root\Viaduct";
/// Each Device's runtime child (ADR 0013).
pub const ENDPOINT_HARDWARE_ID: &str = r"Viaduct\Endpoint";

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

/// An INF file's text: UTF-16LE when it starts with that byte-order mark, else
/// 8-bit text (INFs are ASCII or UTF-8 otherwise).
pub fn decode_inf(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => String::from_utf16_lossy(&utf16le_units(rest)),
        _ => String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
            .into_owned(),
    }
}

/// UTF-16LE bytes as code units; an odd trailing byte is ignored.
fn utf16le_units(bytes: &[u8]) -> Vec<u16> {
    let (pairs, _odd) = bytes.as_chunks::<2>();
    pairs.iter().map(|&pair| u16::from_le_bytes(pair)).collect()
}

/// Whether an INF installs devices with `hardware_id`: a models line
/// `<description> = <install section>, <hardware ID>[, <compatible IDs>]` lists
/// it. Comments (after `;`) are ignored; the match ignores case, as PnP's does.
pub fn inf_lists_hardware_id(text: &str, hardware_id: &str) -> bool {
    text.lines().any(|line| {
        let line = line.split(';').next().unwrap_or("");
        let Some((_, values)) = line.split_once('=') else {
            return false;
        };
        values.split(',').skip(1).any(|value| {
            value
                .trim()
                .trim_matches('"')
                .eq_ignore_ascii_case(hardware_id)
        })
    })
}

#[cfg(windows)]
pub use setup::{install, uninstall};

#[cfg(not(windows))]
pub fn install(_package: &str) -> Result<(), String> {
    Err("install is only available on Windows".to_string())
}

#[cfg(not(windows))]
pub fn uninstall() -> Result<(), String> {
    Err("uninstall is only available on Windows".to_string())
}

#[cfg(windows)]
mod setup;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_and_endpoint_ids_match_the_inf() {
        let inf = include_str!("../../../platforms/windows/driver/viaduct.inf");
        assert!(inf_lists_hardware_id(inf, ROOT_HARDWARE_ID));
        assert!(inf_lists_hardware_id(inf, ENDPOINT_HARDWARE_ID));
        assert_ne!(ROOT_HARDWARE_ID, ENDPOINT_HARDWARE_ID);
    }

    #[test]
    fn multi_sz_round_trips() {
        let bytes = to_multi_sz(&[r"Root\Viaduct", "Second"]);
        assert_eq!(bytes.len(), (12 + 1 + 6 + 1 + 1) * 2);
        assert_eq!(&bytes[bytes.len() - 4..], &[0, 0, 0, 0]);
        assert_eq!(parse_multi_sz(&bytes), vec![r"Root\Viaduct", "Second"]);
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
        let ids = vec![r"ROOT\VIADUCT".to_string(), "Other".to_string()];
        assert!(has_hardware_id(&ids, r"Root\Viaduct"));
        assert!(!has_hardware_id(&ids, r"Root\Viaduct2"));
        assert!(!has_hardware_id(&[], r"Root\Viaduct"));
    }

    #[test]
    fn inf_models_lines_are_matched_not_other_mentions() {
        let inf = "; Root\\Viaduct in a comment\n\
                   [Strings]\n\
                   Desc = \"Root\\Viaduct device\"\n\
                   Section = Root\\Viaduct\n";
        assert!(!inf_lists_hardware_id(inf, r"Root\Viaduct"));
        let models = "[Models.NTamd64]\n%Desc% = Install, root\\viaduct ; the root\n";
        assert!(inf_lists_hardware_id(models, r"Root\Viaduct"));
        let compatible = "%Desc% = Install, Other\\Id, \"Root\\Viaduct\"\n";
        assert!(inf_lists_hardware_id(compatible, r"Root\Viaduct"));
    }

    #[test]
    fn infs_decode_from_utf16_utf8_and_ascii() {
        let text = "%D% = S, Root\\Viaduct";
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_inf(&utf16), text);
        let mut utf8 = b"\xEF\xBB\xBF".to_vec();
        utf8.extend(text.as_bytes());
        assert_eq!(decode_inf(&utf8), text);
        assert_eq!(decode_inf(text.as_bytes()), text);
    }
}
