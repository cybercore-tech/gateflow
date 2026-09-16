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

    /// Building the internal Tokio runtime used for the one-shot netlink
    /// setup (bringing `lo` up, applying chaos) failed.
    Runtime(std::io::Error),

    /// A netlink operation (bringing an interface up, applying `tc netem`)
    /// failed.
    Netlink(nlink::Error),

    /// Creating, reading, or writing one of the coordination pipes
    /// [`crate::veth::fork_veth_pair`] uses to sequence two forked
    /// namespaces' setup failed.
    Pipe(nix::Error),
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
            Error::Runtime(err) => write!(f, "failed to build the netlink setup runtime: {err}"),
            Error::Netlink(err) => write!(f, "netlink operation failed: {err}"),
            Error::Pipe(err) => write!(f, "coordination pipe failed: {err}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Namespace(err) | Error::Fork(err) | Error::Wait(err) | Error::Pipe(err) => {
                Some(err)
            }
            Error::IdMap { source, .. } => Some(source),
            Error::Runtime(source) => Some(source),
            Error::Netlink(err) => Some(err),
            Error::ChildTerminated(_) => None,
        }
    }
}
