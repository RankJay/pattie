//! The stream's one format as a SPA POD, built for the Device's rate and channels.

use super::*;

/// `enum spa_audio_channel` (`spa/param/audio/raw.h`).
const MONO: u32 = 2;
const FL: u32 = 3;
const FR: u32 = 4;
const FC: u32 = 5;
const LFE: u32 = 6;
const SL: u32 = 7;
const SR: u32 = 8;
const RC: u32 = 11;
const RL: u32 = 12;
const RR: u32 = 13;

/// The channel positions for 1 to 8 channels: the standard layouts, in the
/// order Windows' channel masks give them (ADR 0023): mono, stereo, 3.0, quad,
/// 5.0, 5.1, 6.1, 7.1 surround.
const LAYOUTS: [&[u32]; 8] = [
    &[MONO],
    &[FL, FR],
    &[FL, FR, FC],
    &[FL, FR, RL, RR],
    &[FL, FR, FC, RL, RR],
    &[FL, FR, FC, LFE, RL, RR],
    &[FL, FR, FC, LFE, RL, RR, RC],
    &[FL, FR, FC, LFE, RL, RR, SL, SR],
];

/// The format pod for a Device at `rate` with `channels` (1 to 8, as the core
/// checked at attach).
pub(crate) fn format_pod(rate: u32, channels: u16) -> Pod {
    let positions = LAYOUTS[usize::from(channels.clamp(1, 8)) - 1];
    let mut words = [0u32; 48];
    let body = [
        0x4_0003,
        3, // SPA_TYPE_OBJECT_Format, SPA_PARAM_EnumFormat
        1,
        0,
        4,
        3,
        1,
        0, // SPA_FORMAT_mediaType: Id SPA_MEDIA_TYPE_audio
        2,
        0,
        4,
        3,
        1,
        0, // SPA_FORMAT_mediaSubtype: Id SPA_MEDIA_SUBTYPE_raw
        0x1_0001,
        0,
        4,
        3,
        0x11b,
        0, // SPA_FORMAT_AUDIO_format: Id SPA_AUDIO_FORMAT_F32_LE
        0x1_0003,
        0,
        4,
        4,
        rate,
        0, // SPA_FORMAT_AUDIO_rate: Int
        0x1_0004,
        0,
        4,
        4,
        u32::from(channels),
        0, // SPA_FORMAT_AUDIO_channels: Int
    ];
    words[2..2 + body.len()].copy_from_slice(&body);
    let mut len = 2 + body.len();
    // SPA_FORMAT_AUDIO_position: Array (13) of Id (child size 4, type 3).
    #[expect(
        clippy::cast_possible_truncation,
        reason = "at most 8 positions: a few dozen bytes"
    )]
    let array_bytes = (8 + 4 * positions.len()) as u32;
    words[len..len + 6].copy_from_slice(&[0x1_0005, 0, array_bytes, 13, 4, 3]);
    len += 6;
    words[len..len + positions.len()].copy_from_slice(positions);
    len += positions.len().next_multiple_of(2); // padded to 8 bytes with zeros
    #[expect(
        clippy::cast_possible_truncation,
        reason = "at most 48 words: 192 bytes"
    )]
    let object_bytes = (len * 4 - 8) as u32;
    words[..2].copy_from_slice(&[object_bytes, 15]); // Object (SPA_TYPE_Object)
    Pod { words, len }
}
