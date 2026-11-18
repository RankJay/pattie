//! An Owner's conversation with a Backend (ADR 0003, ADR 0012): Hello and the
//! version it settles on, then typed commands. It owns the wire format: command
//! encoding, the reply envelope, reply decoding and the map from driver statuses to
//! `Error`, so a new command (rename, list, later minors: ADR 0017) is one method
//! here rather than more inline protocol in `Device`.

use crate::Error;
use crate::abi::{
    self, CreateDeviceReply, CreateDeviceRequest, HelloReply, HelloRequest, ReplyEnvelope,
};
use crate::backend::{Backend, Connection};
use crate::ring::RingMemory;
use std::mem::size_of;

/// One open handle on a Backend: one Owner. Dropping it closes the handle, and the
/// Backend removes the Devices this Owner created (ADR 0005).
pub(crate) struct Owner {
    connection: Box<dyn Connection>,
    /// The ABI minor both sides speak: the lower of the SDK's and the driver's
    /// (ADR 0003). Nothing depends on it at 1.0; the first command added in a later
    /// minor checks it.
    #[expect(
        dead_code,
        reason = "read by the first command that needs a later minor (ADR 0017)"
    )]
    minor: u16,
}

impl Owner {
    /// Opens an Owner connection on `backend` and says Hello.
    pub(crate) fn open(backend: &dyn Backend) -> Result<Owner, Error> {
        let mut connection = backend.connect()?;
        let hello = HelloRequest {
            major: abi::ABI_MAJOR,
            minor: abi::ABI_MINOR,
        };
        let reply: HelloReply =
            command(connection.as_mut(), abi::CMD_HELLO, abi::bytes_of(&hello))?;
        if reply.major != abi::ABI_MAJOR {
            return Err(Error::VersionMismatch);
        }
        #[expect(
            clippy::unnecessary_min_or_max,
            reason = "the SDK's minor is 0 at ABI 1.0; from 1.1 on this picks the lower one"
        )]
        let minor = reply.minor.min(abi::ABI_MINOR);
        Ok(Owner { connection, minor })
    }

    /// Creates a Device of `kind` (`abi::KIND_*`) named `name`, its audio in
    /// `ring`, which must outlive this session (the Backend uses it until the
    /// session closes). The system's side rings `wake` (a `Doorbell`'s raw value,
    /// which must also outlive this session) after each pass over the Ring.
    /// Returns the Device ID.
    pub(crate) fn create_device(
        &mut self,
        kind: u32,
        name: &str,
        ring: &RingMemory,
        wake: u64,
    ) -> Result<u32, Error> {
        let mut request = CreateDeviceRequest {
            kind,
            name_len: 0,
            name: [0; abi::DEVICE_NAME_MAX],
            reserved: 0,
            ring_address: ring.address(),
            ring_bytes: ring.len() as u64,
            wake_event: wake,
        };
        let units: Vec<u16> = name.encode_utf16().collect();
        let n = units.len().min(abi::DEVICE_NAME_MAX);
        request.name[..n].copy_from_slice(&units[..n]);
        // The driver rejects anything over 63 units; saturating keeps it rejected.
        request.name_len = u32::try_from(units.len()).unwrap_or(u32::MAX);
        let reply: CreateDeviceReply = command(
            self.connection.as_mut(),
            abi::CMD_CREATE_DEVICE,
            abi::bytes_of(&request),
        )?;
        Ok(reply.device_id)
    }
}

impl Owner {
    /// Removes Device `id`: a command the SDK itself never sends (a Device goes
    /// when its session closes), for tests of a Backend's side of it.
    #[cfg(test)]
    pub(crate) fn remove_device(&mut self, id: u32) -> Result<(), Error> {
        let request = abi::RemoveDeviceRequest { device_id: id };
        command::<()>(
            self.connection.as_mut(),
            abi::CMD_REMOVE_DEVICE,
            abi::bytes_of(&request),
        )
    }
}

/// Sends one command and decodes its reply as `R`, a plain ABI struct.
fn command<R: Copy + Default>(
    connection: &mut dyn Connection,
    command: u32,
    input: &[u8],
) -> Result<R, Error> {
    let mut reply = R::default();
    let reply_bytes = abi::bytes_of_mut(&mut reply);
    let envelope_len = size_of::<ReplyEnvelope>();
    let mut output = vec![0u8; envelope_len + reply_bytes.len()];
    let written = connection.send(abi::ioctl(command), input, &mut output)?;
    if written < envelope_len {
        return Err(Error::Protocol(abi::ERR_BAD_REQUEST));
    }
    let status = output
        .first_chunk::<4>()
        .map(|bytes| u32::from_ne_bytes(*bytes))
        .ok_or(Error::Protocol(abi::ERR_BAD_REQUEST))?;
    if status != abi::OK {
        return Err(error_from_status(status));
    }
    if written < envelope_len + reply_bytes.len() {
        return Err(Error::Protocol(abi::ERR_BAD_REQUEST));
    }
    reply_bytes.copy_from_slice(&output[envelope_len..envelope_len + reply_bytes.len()]);
    Ok(reply)
}

fn error_from_status(status: abi::Status) -> Error {
    match status {
        abi::ERR_VERSION => Error::VersionMismatch,
        abi::ERR_BAD_NAME => Error::BadName,
        abi::ERR_DUPLICATE_NAME => Error::DuplicateName,
        abi::ERR_TOO_MANY_DEVICES => Error::TooManyDevices,
        abi::ERR_SYSTEM => Error::System,
        other => Error::Protocol(other),
    }
}
