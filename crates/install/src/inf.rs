//! Reading the driver's INF: its text, its hardware IDs and its version.

use super::{DriverVersion, parse_driver_version, utf16le_units};

/// An INF file's text: UTF-16LE when it starts with that byte-order mark, else
/// 8-bit text (INFs are ASCII or UTF-8 otherwise).
pub fn decode_inf(bytes: &[u8]) -> String {
    match bytes {
        [0xFF, 0xFE, rest @ ..] => String::from_utf16_lossy(&utf16le_units(rest)),
        _ => String::from_utf8_lossy(bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes))
            .into_owned(),
    }
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

/// The version in an INF's `DriverVer = date,a.b.c.d` line.
pub fn inf_driver_version(text: &str) -> Option<DriverVersion> {
    text.lines().find_map(|line| {
        let line = line.split(';').next().unwrap_or("");
        let (key, value) = line.split_once('=')?;
        if !key.trim().eq_ignore_ascii_case("DriverVer") {
            return None;
        }
        parse_driver_version(value.split_once(',')?.1)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_versions_read_from_the_inf_and_the_device() {
        let inf = "[Version]

Signature=\"$WINDOWS NT$\"

DriverVer = 11/15/2026,0.1.2.31

";
        assert_eq!(inf_driver_version(inf), Some([0, 1, 2, 31]));
        assert_eq!(
            inf_driver_version("DriverVer=01/02/2026, 1.2.3.4 ; note"),
            Some([1, 2, 3, 4])
        );
        assert_eq!(
            inf_driver_version(
                "[Version]
Class=MEDIA
"
            ),
            None
        );
        assert_eq!(parse_driver_version("0.1.1.30"), Some([0, 1, 1, 30]));
        assert_eq!(parse_driver_version("0.1.1"), None);
        assert_eq!(parse_driver_version("0.1.1.30.5"), None);
        // Only the DriverVer key, not another with the same shape before it.
        assert_eq!(
            inf_driver_version(
                "OtherVer = 01/01/2020,9.9.9.9
DriverVer = 11/15/2026,0.1.2.31
"
            ),
            Some([0, 1, 2, 31])
        );
        assert_eq!(parse_driver_version("0.1.1.70000"), None);
    }

    #[test]
    fn inf_models_lines_are_matched_not_other_mentions() {
        let inf = "; Root\\Pattie in a comment\n\
                   [Strings]\n\
                   Desc = \"Root\\Pattie device\"\n\
                   Section = Root\\Pattie\n";
        assert!(!inf_lists_hardware_id(inf, r"Root\Pattie"));
        let models = "[Models.NTamd64]\n%Desc% = Install, root\\pattie ; the root\n";
        assert!(inf_lists_hardware_id(models, r"Root\Pattie"));
        let compatible = "%Desc% = Install, Other\\Id, \"Root\\Pattie\"\n";
        assert!(inf_lists_hardware_id(compatible, r"Root\Pattie"));
    }

    #[test]
    fn infs_decode_from_utf16_utf8_and_ascii() {
        let text = "%D% = S, Root\\Pattie";
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
        assert_eq!(decode_inf(&utf16), text);
        let mut utf8 = b"\xEF\xBB\xBF".to_vec();
        utf8.extend(text.as_bytes());
        assert_eq!(decode_inf(&utf8), text);
        assert_eq!(decode_inf(text.as_bytes()), text);
    }
}
