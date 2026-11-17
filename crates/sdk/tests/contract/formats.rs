//! Device formats and buffer sizes, accepted and refused.

use super::*;

contract_suite!(
    formats_outside_three_rates_and_eight_channels_are_rejected,
    the_three_rates_and_one_to_eight_channels_are_accepted,
    buffer_sizes_outside_the_allowed_powers_of_two_are_rejected,
    a_device_reports_its_format,
);

fn formats_outside_three_rates_and_eight_channels_are_rejected(backend: &dyn BackendUnderTest) {
    let results = [
        (22_050, 1),
        (44_101, 2),
        (192_000, 1),
        (48_000, 0),
        (48_000, 9),
    ]
    .map(|(rate, channels)| {
        builder(backend)
            .name("Agent Mic")
            .format(rate, channels)
            .create()
            .err()
    });

    assert_eq!(results, [Some(pattie::Error::UnsupportedFormat); 5]);
}

fn the_three_rates_and_one_to_eight_channels_are_accepted(backend: &dyn BackendUnderTest) {
    let refused: Vec<_> = [44_100, 48_000, 96_000]
        .into_iter()
        .flat_map(|rate| (1..=8u16).map(move |channels| (rate, channels)))
        .filter(|&(rate, channels)| {
            let mic = builder(backend)
                .name(format!("Mic {rate} {channels}"))
                .format(rate, channels)
                .create();
            mic.is_err()
        })
        .collect();

    assert_eq!(refused, []);
}

fn buffer_sizes_outside_the_allowed_powers_of_two_are_rejected(backend: &dyn BackendUnderTest) {
    let results = [0, 128, 1000, 131_072, 1 << 31].map(|frames| {
        builder(backend)
            .name("Agent Mic")
            .buffer_frames(frames)
            .create()
            .err()
    });

    assert_eq!(results, [Some(pattie::Error::BadBufferSize); 5]);
}

fn a_device_reports_its_format(backend: &dyn BackendUnderTest) {
    let default = builder(backend).name("Agent Mic").create().unwrap();
    let stereo = builder(backend)
        .name("Stereo Mic")
        .format(44_100, 2)
        .create()
        .unwrap();

    assert_eq!(default.format(), pattie::AudioFormat::DEVICE);
    assert_eq!(
        stereo.format(),
        pattie::AudioFormat {
            rate: 44_100,
            channels: 2,
            encoding: pattie::Encoding::F32
        }
    );
}
