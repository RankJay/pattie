//! The rules every command sequence must follow (a model of the Core), and the property
//! test that checks the Core against them step by step.

use super::*;

/// What the rules say the core's state is.
#[derive(Default)]
struct Model {
    owners: Vec<OwnerRef>,
    open: Vec<bool>,
    greeted: Vec<bool>,
    minor: Vec<u16>,
    devices: Vec<LiveDevice>,
    next_id: u32,
}

impl Model {
    /// Index of the `pick`-th open Owner, if any is open.
    fn open_owner(&self, pick: u8) -> Option<usize> {
        let open: Vec<usize> = (0..self.owners.len()).filter(|&i| self.open[i]).collect();
        (!open.is_empty()).then(|| open[pick as usize % open.len()])
    }

    fn hello(&mut self, owner: usize, major: u16, minor: u16) -> pattie_status {
        if u32::from(major) != PATTIE_ABI_MAJOR {
            return PATTIE_ERR_VERSION;
        }
        self.greeted[owner] = true;
        self.minor[owner] = minor.min(PATTIE_ABI_MINOR as u16);
        PATTIE_OK
    }

    fn create(
        &mut self,
        owner: usize,
        kind: u32,
        name: &[u16],
        name_len: u32,
        ring: RingChoice,
    ) -> pattie_status {
        if !self.greeted[owner] {
            return PATTIE_ERR_HELLO_REQUIRED;
        }
        if kind != PATTIE_KIND_MICROPHONE && kind != PATTIE_KIND_SPEAKER {
            return PATTIE_ERR_BAD_KIND;
        }
        let valid_name = (1..=PATTIE_DEVICE_NAME_MAX).contains(&name_len)
            && name
                .iter()
                .all(|&unit| !(unit < 0x20 || (0x7F..=0x9F).contains(&unit)));
        if !valid_name {
            return PATTIE_ERR_BAD_NAME;
        }
        if self
            .devices
            .iter()
            .any(|d| d.kind == kind && d.name == name)
        {
            return PATTIE_ERR_DUPLICATE_NAME;
        }
        if self.devices.len() == PATTIE_MAX_DEVICES as usize {
            return PATTIE_ERR_TOO_MANY_DEVICES;
        }
        if !matches!(ring, RingChoice::Valid) {
            return PATTIE_ERR_BAD_RING;
        }
        self.devices.push(LiveDevice {
            id: self.next_id,
            kind,
            owner: Some(self.owners[owner]),
            name: name.to_vec(),
        });
        self.next_id += 1;
        PATTIE_OK
    }

    fn remove(&mut self, owner: usize, id: u32) -> pattie_status {
        if !self.greeted[owner] {
            return PATTIE_ERR_HELLO_REQUIRED;
        }
        let me = Some(self.owners[owner]);
        match self
            .devices
            .iter()
            .position(|d| id != 0 && d.id == id && d.owner == me)
        {
            Some(i) => {
                self.devices.remove(i);
                PATTIE_OK
            }
            None => PATTIE_ERR_NO_SUCH_DEVICE,
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

    fn raw(&mut self, owner: usize, command: u32, bytes: &[u8]) -> pattie_status {
        if command == PATTIE_CMD_HELLO {
            return match bytes {
                [a, b, c, d] => self.hello(
                    owner,
                    u16::from_le_bytes([*a, *b]),
                    u16::from_le_bytes([*c, *d]),
                ),
                _ => PATTIE_ERR_BAD_REQUEST,
            };
        }
        if !self.greeted[owner] {
            return PATTIE_ERR_HELLO_REQUIRED;
        }
        PATTIE_ERR_BAD_REQUEST // wrong size, or not a command at all
    }
}

/// A successful Hello replies with the driver's version; nothing else here replies.
fn expected_reply(command: u32, status: pattie_status) -> Vec<u8> {
    if command != PATTIE_CMD_HELLO || status != PATTIE_OK {
        return vec![];
    }
    [PATTIE_ABI_MAJOR as u16, PATTIE_ABI_MINOR as u16]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn every_command_sequence_follows_the_rules(ops in prop::collection::vec(op(), 1..80)) {
        let mut driver = Harness::new();
        let mut model = Model { next_id: 1, ..Model::default() };

        for op in ops {
            match op {
                Op::Connect => {
                    model.owners.push(driver.connect());
                    model.open.push(true);
                    model.greeted.push(false);
                    model.minor.push(0);
                }
                Op::Hello { owner, major, minor } => {
                    let Some(i) = model.open_owner(owner) else { continue };
                    let (status, reply) = driver.hello_with(model.owners[i], major, minor, 256);
                    let expected = model.hello(i, major, minor);
                    prop_assert_eq!(status, expected, "Hello {}.{}", major, minor);
                    prop_assert_eq!(reply, expected_reply(PATTIE_CMD_HELLO, status));
                }
                Op::Create { owner, kind, name, ring } => {
                    let Some(i) = model.open_owner(owner) else { continue };
                    let (request, units, name_len) = request_for(kind, name);
                    let expected = model.create(i, kind, &units, name_len, ring);
                    let got = driver.create_device_with(model.owners[i], request, ring_for(ring),
                        256);
                    match got {
                        Ok(id) => {
                            prop_assert_eq!(expected, PATTIE_OK);
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
                    prop_assert_eq!(reply, expected_reply(command, status), "command {}", command);
                }
            }
            prop_assert_eq!(driver.live_devices(), model.devices.clone());
            for (i, &owner) in model.owners.iter().enumerate() {
                let agreed = model.greeted[i].then_some(model.minor[i]);
                prop_assert_eq!(driver.agreed_minor(owner), agreed);
            }
        }
    }
}
