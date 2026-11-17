//! What the command line asks for, parsed and checked before anything runs.

#![forbid(unsafe_code)] // pure logic: the compiler guarantees no unsafe here

mod parse;
#[cfg(test)]
mod tests;

pub use parse::parse;

pub const USAGE: &str = "usage:\n  pattie-cli mic <name> [--tone <hz>] [--rate <hz>] [--channels <n>]\n  pattie-cli speaker <name> [--rate <hz>] [--channels <n>]\n  pattie-cli measure mic|speaker <name> [--seconds <n>] [--rate <hz>] [--channels <n>]\n  pattie-cli measure drift <name> [--seconds <n>] [--ppm <p>] [--uncorrected]\n  pattie-cli measure cable <render endpoint> <capture endpoint> [--seconds <n>]\n  pattie-cli install <package folder or pattie.inf>\n  pattie-cli uninstall";

/// The rate and channels a Device runs at (ADR 0023): 48 kHz mono by default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceFormat {
    pub rate: u32,
    pub channels: u16,
}

impl Default for DeviceFormat {
    fn default() -> Self {
        DeviceFormat {
            rate: 48_000,
            channels: 1,
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum Command<'a> {
    Mic {
        name: &'a str,
        hz: f32,
        format: DeviceFormat,
    },
    Speaker {
        name: &'a str,
        format: DeviceFormat,
    },
    Measure {
        direction: &'a str,
        name: &'a str,
        seconds: u64,
        format: DeviceFormat,
    },
    /// Another virtual device as a baseline (VB-CABLE, say): clicks rendered into
    /// one existing endpoint, found in another, each named by part of its name.
    Cable {
        render: &'a str,
        capture: &'a str,
        seconds: u64,
    },
    /// A live source `ppm` off the device, through a `LiveWriter`, logged.
    Drift {
        name: &'a str,
        seconds: u64,
        ppm: f64,
        correct: bool,
    },
    Install {
        package: &'a str,
    },
    Uninstall,
}
