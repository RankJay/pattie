//! Seam A, by property (ADR 0008): random sequences of commands from several apps,
//! well-formed or not, checked step by step against a model of the rules.
//!
//! After every step the core's status must be the model's, and its live Device
//! table must equal the model's exactly: IDs never reused, at most 8 Devices, no
//! two of one kind with the same name, each owned by the app that created it.

use proptest::prelude::*;
use std::mem::size_of;
use viaduct_core_tests::ffi::*;
use viaduct_core_tests::{Driver, LiveDevice, OwnerRef, RingMemory, create_device_request};

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
    let create_len = size_of::<vd_create_device_request>();
    let remove_len = size_of::<vd_remove_device_request>();
    prop_oneof![
        1 => Just(Op::Connect),
        2 => (any::<u8>(), prop_oneof![
                // The served version, and the values just past each boundary.
                6 => Just((VIADUCT_ABI_MAJOR as u16, VIADUCT_ABI_MINOR as u16)),
                1 => Just((VIADUCT_ABI_MAJOR as u16, 0)),
                1 => Just((VIADUCT_ABI_MAJOR as u16, VIADUCT_ABI_MINOR as u16 + 1)),
                1 => Just((VIADUCT_ABI_MAJOR as u16 + 1, VIADUCT_ABI_MINOR as u16)),
                1 => any::<(u16, u16)>(),
            ])
            .prop_map(|(owner, (major, minor))| Op::Hello { owner, major, minor }),
        6 => (
                any::<u8>(),
                prop_oneof![4 => Just(VD_KIND_MICROPHONE), 4 => Just(VD_KIND_SPEAKER), 1 => 0..=4u32],
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
                prop_oneof![Just(VD_CMD_HELLO), Just(VD_CMD_CREATE_DEVICE), Just(VD_CMD_REMOVE_DEVICE), any::<u32>()],
                prop::collection::vec(any::<u8>(), 0..200),
            )
            // A random request of exactly the right size can't be modelled simply;
            // those are what the Create and Remove ops are for.
            .prop_filter("well-sized create/remove", move |(_, command, bytes)| {
                !(*command == VD_CMD_CREATE_DEVICE && bytes.len() == create_len)
                    && !(*command == VD_CMD_REMOVE_DEVICE && bytes.len() == remove_len)
            })
            .prop_map(|(owner, command, bytes)| Op::Raw { owner, command, bytes }),
    ]
}

/// What the rules say the core's state is.
#[derive(Default)]
struct Model {
    owners: Vec<OwnerRef>,
    open: Vec<bool>,
    greeted: Vec<bool>,
    devices: Vec<LiveDevice>,
    next_id: u32,
}

impl Model {
    /// Index of the `pick`-th open Owner, if any is open.
    fn open_owner(&self, pick: u8) -> Option<usize> {
        let open: Vec<usize> = (0..self.owners.len()).filter(|&i| self.open[i]).collect();
        (!open.is_empty()).then(|| open[pick as usize % open.len()])
    }

    fn hello(&mut self, owner: usize, major: u16, minor: u16) -> vd_status {
        if u32::from(major) != VIADUCT_ABI_MAJOR || u32::from(minor) > VIADUCT_ABI_MINOR {
            return VD_ERR_VERSION;
        }
        self.greeted[owner] = true;
        VD_OK
    }

    fn create(
        &mut self,
        owner: usize,
        kind: u32,
        name: &[u16],
        name_len: u32,
        ring: RingChoice,
    ) -> vd_status {
        if !self.greeted[owner] {
            return VD_ERR_HELLO_REQUIRED;
        }
        if kind != VD_KIND_MICROPHONE && kind != VD_KIND_SPEAKER {
            return VD_ERR_BAD_KIND;
        }
        let valid_name = (1..=VD_DEVICE_NAME_MAX).contains(&name_len)
            && name
                .iter()
                .all(|&unit| !(unit < 0x20 || (0x7F..=0x9F).contains(&unit)));
        if !valid_name {
            return VD_ERR_BAD_NAME;
        }
        if self
            .devices
            .iter()
            .any(|d| d.kind == kind && d.name == name)
        {
            return VD_ERR_DUPLICATE_NAME;
        }
        if self.devices.len() == VD_MAX_DEVICES as usize {
            return VD_ERR_TOO_MANY_DEVICES;
        }
        if !matches!(ring, RingChoice::Valid) {
            return VD_ERR_BAD_RING;
        }
        self.devices.push(LiveDevice {
            id: self.next_id,
            kind,
            owner: Some(self.owners[owner]),
            name: name.to_vec(),
        });
        self.next_id += 1;
        VD_OK
    }

    fn remove(&mut self, owner: usize, id: u32) -> vd_status {
        if !self.greeted[owner] {
            return VD_ERR_HELLO_REQUIRED;
        }
        let me = Some(self.owners[owner]);
        match self
            .devices
            .iter()
            .position(|d| id != 0 && d.id == id && d.owner == me)
        {
            Some(i) => {
                self.devices.remove(i);
                VD_OK
            }
            None => VD_ERR_NO_SUCH_DEVICE,
        }
    }

    fn close(&mut self, owner: usize) -> Vec<u32> {
        let me = Some(self.owners[owner]);
        let mut removed: Vec<u32> = self
            .devices
            .iter()
            .filter(|d| d.owner == me)
            .map(|d| d.id)
            .collect();
        self.devices.retain(|d| d.owner != me);
        self.open[owner] = false;
        removed.sort();
        removed
    }

    fn raw(&mut self, owner: usize, command: u32, bytes: &[u8]) -> vd_status {
        if command == VD_CMD_HELLO {
            return match bytes {
                [a, b, c, d] => self.hello(
                    owner,
                    u16::from_le_bytes([*a, *b]),
                    u16::from_le_bytes([*c, *d]),
                ),
                _ => VD_ERR_BAD_REQUEST,
            };
        }
        if !self.greeted[owner] {
            return VD_ERR_HELLO_REQUIRED;
        }
        VD_ERR_BAD_REQUEST // wrong size, or not a command at all
    }
}

/// The request and the name the model sees for a `NAMES` index (or the specials).
fn request_for(kind: u32, name: usize) -> (vd_create_device_request, Vec<u16>, u32) {
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

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn every_command_sequence_follows_the_rules(ops in prop::collection::vec(op(), 1..80)) {
        let mut driver = Driver::new();
        let mut model = Model { next_id: 1, ..Model::default() };

        for op in ops {
            match op {
                Op::Connect => {
                    model.owners.push(driver.connect());
                    model.open.push(true);
                    model.greeted.push(false);
                }
                Op::Hello { owner, major, minor } => {
                    let Some(i) = model.open_owner(owner) else { continue };
                    let status = driver.hello(model.owners[i], major, minor);
                    prop_assert_eq!(status, model.hello(i, major, minor), "Hello {}.{}", major, minor);
                }
                Op::Create { owner, kind, name, ring } => {
                    let Some(i) = model.open_owner(owner) else { continue };
                    let (request, units, name_len) = request_for(kind, name);
                    let expected = model.create(i, kind, &units, name_len, ring);
                    let got = driver.create_device_with(model.owners[i], request, ring_for(ring), 256);
                    match got {
                        Ok(id) => {
                            prop_assert_eq!(expected, VD_OK);
                            prop_assert_eq!(Some(id), model.devices.last().map(|d| d.id));
                        }
                        Err(status) => prop_assert_eq!(status, expected),
                    }
                }
                Op::Remove { owner, id } => {
                    let Some(i) = model.open_owner(owner) else { continue };
                    prop_assert_eq!(driver.remove_device(model.owners[i], id), model.remove(i, id));
                }
                Op::Close { owner } => {
                    let Some(i) = model.open_owner(owner) else { continue };
                    prop_assert_eq!(driver.close(model.owners[i]), model.close(i));
                }
                Op::Raw { owner, command, bytes } => {
                    let Some(i) = model.open_owner(owner) else { continue };
                    let (status, reply) = driver.send(model.owners[i], command, &bytes);
                    prop_assert_eq!(status, model.raw(i, command, &bytes));
                    prop_assert!(reply.is_empty(), "no reply for command {}", command);
                }
            }
            prop_assert_eq!(driver.live_devices(), model.devices.clone());
        }
    }
}
