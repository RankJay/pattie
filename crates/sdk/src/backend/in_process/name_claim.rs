//! Holding a Device's name across apps (ADR 0018 amendment).

use crate::Error;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

/// A Device's name, held across apps for as long as the Device lives (ADR 0018):
/// a lock (`flock`) on a file named after it in this user's runtime directory.
/// The kernel lets one open file hold the lock at a time, so two apps can't both
/// win, and releases it the moment the process exits, crash included. The folder
/// is this user's and private (0700), so no other user can take a name. (Until
/// 2026-11-18 the claim was an abstract Unix socket, which any local user could
/// bind first: the user ID was only text in its name.)
pub(super) struct Claim {
    path: PathBuf,
    /// Locked: holding it is the claim, and dropping it releases it.
    _lock: File,
}

impl Drop for Claim {
    fn drop(&mut self) {
        // Removed while still locked (the lock goes with `_lock`, after this): an
        // app that opened it meanwhile finds, once it holds the lock, that the
        // path no longer leads to its file, and tries again (`lock`).
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Claims `node_name` in `namespace` (`pattie`, or a test's own); a name another
/// process holds is `DuplicateName`.
pub(super) fn claim(namespace: &str, node_name: &str) -> Result<Claim, Error> {
    // /proc/self belongs to the process's effective user.
    let uid = std::fs::metadata("/proc/self").map_err(os_error)?.uid();
    let path = claims_folder(uid)?.join(format!("{namespace}.{node_name}.lock"));
    lock(path)
}

fn os_error(error: std::io::Error) -> Error {
    Error::Os(error.raw_os_error().unwrap_or(0))
}

/// `$XDG_RUNTIME_DIR/pattie`, made if missing, and checked to be this user's own
/// and private: a runtime directory set to somewhere shared (`/tmp`, say) would
/// otherwise let another user make it first. No runtime directory means no
/// PipeWire session either (its socket lives there): `AudioServerUnavailable`.
fn claims_folder(uid: u32) -> Result<PathBuf, Error> {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or(Error::AudioServerUnavailable)?;
    let folder = PathBuf::from(runtime).join("pattie");
    match std::fs::DirBuilder::new().mode(0o700).create(&folder) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(os_error(error)),
    }
    let folder_meta = std::fs::symlink_metadata(&folder).map_err(os_error)?;
    if !folder_meta.is_dir() || folder_meta.uid() != uid || folder_meta.mode() & 0o077 != 0 {
        return Err(Error::Os(libc::EACCES));
    }
    Ok(folder)
}

/// Locks the file at `path`, making it if missing. Held elsewhere:
/// `DuplicateName`. The holder before may have removed it (`Claim::drop`) between
/// this open and this lock: the lock is then on a file nobody else can find, so
/// it's only ours if the path still leads to the very file locked.
fn lock(path: PathBuf) -> Result<Claim, Error> {
    loop {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(os_error)?;
        // SAFETY: a descriptor `file` owns, open for the whole call.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            let error = std::io::Error::last_os_error();
            return Err(if error.raw_os_error() == Some(libc::EWOULDBLOCK) {
                Error::DuplicateName
            } else {
                os_error(error)
            });
        }
        if same_file(&file, &path)? {
            return Ok(Claim { path, _lock: file });
        }
    }
}

/// Whether `path` still leads to `file`; a missing path is no.
fn same_file(file: &File, path: &Path) -> Result<bool, Error> {
    let held = file.metadata().map_err(os_error)?;
    match std::fs::symlink_metadata(path) {
        Ok(now) => Ok(now.dev() == held.dev() && now.ino() == held.ino()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(os_error(error)),
    }
}
