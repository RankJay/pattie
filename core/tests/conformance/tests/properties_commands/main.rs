//! Seam A, by property (ADR 0008): random sequences of commands from several apps,
//! well-formed or not, checked step by step against a model of the rules.
//!
//! After every step the core's status must be the model's, and its live Device
//! table must equal the model's exactly: IDs never reused, at most 8 Devices, no
//! two of one kind with the same name, each owned by the app that created it. And
//! each connection speaks the minor version Hello agreed (ADR 0003).

use pattie_conformance::ffi::*;

use pattie_conformance::{Harness, LiveDevice, OwnerRef, RingMemory, create_device_request};

use proptest::prelude::*;

use std::mem::size_of;

mod command_rules;

/// Names offered to Create Device: valid, case variants, Unicode, and invalid ones.
const NAMES: &[&str] = &[
    "Agent Mic",
    "agent mic",
    "A",
    "Café",
    "Mic 日本",
    "",
    "Bad\nName",
    "\u{85}",
    "Out",
];

/// Index past `NAMES`: a 63-unit name claiming 64 units (too long).
const TOO_LONG: usize = NAMES.len();

/// Index past that: exactly 63 units (the longest allowed).
const LONGEST: usize = NAMES.len() + 1;

#[derive(Clone, Copy, Debug)]
enum RingChoice {
    Valid,
    Missing,
    TooSmall,
}

#[derive(Clone, Debug)]
enum Op {
    Connect,
    Hello {
        owner: u8,
        major: u16,
        minor: u16,
    },
    Create {
        owner: u8,
        kind: u32,
        name: usize,
        ring: RingChoice,
    },
    Remove {
        owner: u8,
        id: u32,
    },
    Close {
        owner: u8,
    },
    Raw {
        owner: u8,
        command: u32,
        bytes: Vec<u8>,
    },
}

fn op() -> impl Strategy<Value = Op> {
    let create_len = size_of::<pattie_create_device_request>();
    let remove_len = size_of::<pattie_remove_device_request>();
    prop_oneof![
        1 => Just(Op::Connect),
        2 => (any::<u8>(), prop_oneof![
                // The served version, and the values just past each boundary.
                6 => Just((PATTIE_ABI_MAJOR as u16, PATTIE_ABI_MINOR as u16)),
                1 => Just((PATTIE_ABI_MAJOR as u16, 0)),
                1 => Just((PATTIE_ABI_MAJOR as u16, PATTIE_ABI_MINOR as u16 + 1)),
                1 => Just((PATTIE_ABI_MAJOR as u16 + 1, PATTIE_ABI_MINOR as u16)),
                1 => Just((PATTIE_ABI_MAJOR as u16 - 1, PATTIE_ABI_MINOR as u16)),
                1 => any::<(u16, u16)>(),
            ])
            .prop_map(|(owner, (major, minor))| Op::Hello { owner, major, minor }),
        6 => (
                any::<u8>(),
                prop_oneof![4 => Just(PATTIE_KIND_MICROPHONE), 4 => Just(PATTIE_KIND_SPEAKER), 1 => 0..=4u32],
                0..=LONGEST,
                prop_oneof![
                    8 => Just(RingChoice::Valid),
                    1 => Just(RingChoice::Missing),
                    1 => Just(RingChoice::TooSmall),
                ],
            )
            .prop_map(|(owner, kind, name, ring)| Op::Create { owner, kind, name, ring }),
        3 => (any::<u8>(), 0..=12u32).prop_map(|(owner, id)| Op::Remove { owner, id }),
        1 => any::<u8>().prop_map(|owner| Op::Close { owner }),
        2 => (
                any::<u8>(),
                prop_oneof![Just(PATTIE_CMD_HELLO), Just(PATTIE_CMD_CREATE_DEVICE), Just(PATTIE_CMD_REMOVE_DEVICE), any::<u32>()],
                prop::collection::vec(any::<u8>(), 0..200),
            )
            // A random request of exactly the right size can't be modelled simply;
            // those are what the Create and Remove ops are for.
            .prop_filter("well-sized create/remove", move |(_, command, bytes)| {
                !(*command == PATTIE_CMD_CREATE_DEVICE && bytes.len() == create_len)
                    && !(*command == PATTIE_CMD_REMOVE_DEVICE && bytes.len() == remove_len)
            })
            .prop_map(|(owner, command, bytes)| Op::Raw { owner, command, bytes }),
    ]
}

/// The request and the name the model sees for a `NAMES` index (or the specials).
fn request_for(kind: u32, name: usize) -> (pattie_create_device_request, Vec<u16>, u32) {
    if name == TOO_LONG || name == LONGEST {
        let mut request = create_device_request(kind, &"x".repeat(63));
        let units = vec![u16::from(b'x'); 63];
        if name == TOO_LONG {
            request.name_len = 64;
        }
        let name_len = request.name_len;
        return (request, units, name_len);
    }
    let request = create_device_request(kind, NAMES[name]);
    let units: Vec<u16> = NAMES[name].encode_utf16().collect();
    (request, units, request.name_len)
}

fn ring_for(choice: RingChoice) -> Option<RingMemory> {
    match choice {
        RingChoice::Valid => Some(RingMemory::new(256)),
        RingChoice::Missing => None,
        RingChoice::TooSmall => Some(RingMemory::with_claim(256, RingMemory::bytes_for(256) - 4)),
    }
}
