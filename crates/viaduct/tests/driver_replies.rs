//! How the SDK reads the driver's replies when they aren't a success: statuses the
//! real core never sends this SDK (a driver of another ABI version, Windows
//! refusing the endpoint) and malformed replies. A scripted connection stands in
//! for the driver, answering every command with one chosen reply.

use std::sync::Arc;
use viaduct::abi;
use viaduct::transport::{Connection, Transport};
use viaduct::{Device, Error};

/// What the scripted driver answers.
#[derive(Clone, Copy)]
enum Reply {
    /// This status for Hello; success for everything else.
    Hello(abi::Status),
    /// Success for Hello, this status for Create Device.
    Create(abi::Status),
    /// Success for Hello; for Create Device, only this many bytes.
    ShortCreate(usize),
}

struct Scripted(Reply);

impl Transport for Scripted {
    fn connect(&self) -> Result<Box<dyn Connection>, Error> {
        Ok(Box::new(ScriptedConnection(self.0)))
    }
}

struct ScriptedConnection(Reply);

impl Connection for ScriptedConnection {
    fn ioctl(&mut self, code: u32, _input: &[u8], output: &mut [u8]) -> Result<usize, Error> {
        let hello = code == abi::ioctl(abi::CMD_HELLO);
        let status = match (self.0, hello) {
            (Reply::Hello(status), true) => status,
            (Reply::Create(status), false) => status,
            (Reply::ShortCreate(len), false) => return Ok(len),
            _ => abi::OK,
        };
        output[..4].copy_from_slice(&status.to_ne_bytes());
        Ok(output.len())
    }
}

fn create_with(reply: Reply) -> Result<Device, Error> {
    Device::builder()
        .name("Agent Mic")
        .transport(Arc::new(Scripted(reply)))
        .create()
}

#[test]
fn a_driver_of_another_abi_version_is_a_version_mismatch() {
    let result = create_with(Reply::Hello(abi::ERR_VERSION));

    assert_eq!(result.err(), Some(Error::VersionMismatch));
}

#[test]
fn windows_refusing_the_endpoint_is_a_system_error() {
    let result = create_with(Reply::Create(abi::ERR_SYSTEM));

    assert_eq!(result.err(), Some(Error::System));
}

#[test]
fn a_status_the_sdk_never_expects_is_a_protocol_error_carrying_it() {
    let results = [abi::ERR_BAD_RING, abi::ERR_HELLO_REQUIRED, 999]
        .map(|status| create_with(Reply::Create(status)).err());

    assert_eq!(
        results,
        [
            Some(Error::Protocol(abi::ERR_BAD_RING)),
            Some(Error::Protocol(abi::ERR_HELLO_REQUIRED)),
            Some(Error::Protocol(999)),
        ]
    );
}

#[test]
fn a_reply_shorter_than_its_envelope_is_a_protocol_error() {
    let result = create_with(Reply::ShortCreate(2));

    assert_eq!(result.err(), Some(Error::Protocol(abi::ERR_BAD_REQUEST)));
}
