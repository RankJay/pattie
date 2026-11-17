//! Holding a Device's name across apps (ADR 0018 amendment).

use crate::Error;

/// A Device's name, held across apps for as long as the Device lives (ADR 0018):
/// an abstract Unix socket named after it. The kernel lets one process bind a name
/// at a time, so two apps can't both win, and releases it the moment the process
/// exits, crash included. The user ID is in the name, so users don't block each
/// other.
pub(super) struct Claim {
    /// Held, never used: binding it is the claim, and dropping it releases it.
    _socket: std::os::unix::net::UnixListener,
}

/// Claims `node_name` in `namespace` (`pattie`, or a test's own); a name another
/// process holds is `DuplicateName`.
pub(super) fn claim(namespace: &str, node_name: &str) -> Result<Claim, Error> {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::{SocketAddr, UnixListener};

    let os_error = |error: std::io::Error| Error::Os(error.raw_os_error().unwrap_or(0));
    // /proc/self belongs to the process's effective user.
    let uid = std::fs::metadata("/proc/self").map_err(os_error)?.uid();
    let address = SocketAddr::from_abstract_name(format!("{namespace}/{uid}/{node_name}"))
        .map_err(os_error)?;
    match UnixListener::bind_addr(&address) {
        Ok(socket) => Ok(Claim { _socket: socket }),
        Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => Err(Error::DuplicateName),
        Err(error) => Err(os_error(error)),
    }
}
