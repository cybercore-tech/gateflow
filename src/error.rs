//! Error types returned by `gateflow`.

use std::fmt;

/// Errors that can occur while creating or entering a sandbox.
#[derive(Debug)]
pub enum Error {
    /// A namespace-related syscall failed.
    Namespace(nix::Error),

    /// A `fork(2)` call failed.
    Fork(nix::Error),

    /// A `waitpid(2)` call failed.
    Wait(nix::Error),

    /// The child process did not exit normally (for example, it was
    /// killed by a signal). Carries the raw `waitpid(2)` status for
    /// debugging.
    ChildTerminated(nix::sys::wait::WaitStatus),

    /// Writing one of the `/proc/self/{uid,gid}_map` or
    /// `/proc/self/setgroups` files failed.
    IdMap {
        /// The `/proc/self/...` path that could not be written.
        path: &'static str,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Namespace(err) => write!(f, "namespace syscall failed: {err}"),
            Error::Fork(err) => write!(f, "fork failed: {err}"),
            Error::Wait(err) => write!(f, "waitpid failed: {err}"),
            Error::ChildTerminated(status) => {
                write!(f, "child did not exit normally: {status:?}")
            }
            Error::IdMap { path, source } => write!(f, "failed writing {path}: {source}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Namespace(err) | Error::Fork(err) | Error::Wait(err) => Some(err),
            Error::IdMap { source, .. } => Some(source),
            Error::ChildTerminated(_) => None,
        }
    }
}
