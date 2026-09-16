//! Unprivileged Linux user + network namespace creation.
//!
//! This is the one real, working primitive `gateflow` has today: put the
//! calling process into a fresh network namespace it can configure freely,
//! without needing real root on the host. It's the same trick rootless
//! Podman and `slirp4netns` use — pair `CLONE_NEWUSER` with `CLONE_NEWNET`
//! and map the caller to uid/gid 0 *inside* the new namespace only.
//!
//! # The single-threaded constraint
//!
//! `unshare(2)` with `CLONE_NEWUSER` requires the calling **process** —
//! not just the calling thread — to be single-threaded at the moment of
//! the call. A typical Rust test binary is not: the `cargo test` harness
//! runs many tests concurrently on their own OS threads within one
//! process, so calling [`enter_unprivileged_net_namespace`] directly from
//! a `#[test]` function will generally fail.
//!
//! [`fork_and_enter`] exists to sidestep this: it forks first, and the
//! child of `fork(2)` is guaranteed to start life single-threaded
//! regardless of how many threads the parent had, so entering the
//! namespace immediately after fork is safe.

use std::fs;

use nix::sched::{CloneFlags, unshare};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, fork, getgid, getuid};

use crate::Error;

/// Puts the *current* process into a fresh, unprivileged user + network
/// namespace.
///
/// On success, the calling process is mapped to uid 0 / gid 0 inside the
/// new namespace (and only inside it — this grants no privilege on the
/// host) and owns a network namespace containing nothing but a loopback
/// interface. Wiring it to anything (a veth pair, routes, `tc netem`
/// rules) is not implemented yet.
///
/// # Errors
///
/// Returns [`Error::Namespace`] if `unshare(2)` fails (for example, if
/// unprivileged user namespaces are disabled on this host via
/// `kernel.unprivileged_userns_clone=0`), or [`Error::IdMap`] if writing
/// the `/proc/self/{uid,gid}_map` or `/proc/self/setgroups` files fails.
///
/// # Panics
///
/// Does not panic. It *will* misbehave (per `unshare(2)`) if the calling
/// process is multithreaded — see the [module docs](self) — but that
/// surfaces as an `Err`, not a panic, on every kernel this has been
/// checked against.
pub fn enter_unprivileged_net_namespace() -> Result<(), Error> {
    let uid = getuid();
    let gid = getgid();

    unshare(CloneFlags::CLONE_NEWUSER | CloneFlags::CLONE_NEWNET).map_err(Error::Namespace)?;

    // Order matters: `setgroups` must be denied before `gid_map` can be
    // written by an unprivileged process, or the write fails with EPERM.
    write_proc_self("/proc/self/setgroups", "deny")?;
    write_proc_self("/proc/self/uid_map", &format!("0 {uid} 1"))?;
    write_proc_self("/proc/self/gid_map", &format!("0 {gid} 1"))?;

    Ok(())
}

fn write_proc_self(path: &'static str, contents: &str) -> Result<(), Error> {
    fs::write(path, contents).map_err(|source| Error::IdMap { path, source })
}

/// Forks the current process and enters an unprivileged net namespace in
/// the child, immediately after `fork(2)` — see the [module docs](self)
/// for why the fork is necessary at all.
///
/// `child_fn` runs inside the new namespace and its return value becomes
/// the child process's exit code. The parent blocks in `waitpid(2)` and
/// returns that exit code once the child terminates.
///
/// If entering the namespace itself fails, the child exits with status
/// `111` (a value unlikely to collide with `child_fn`'s own exit codes)
/// without ever running `child_fn`.
///
/// # Errors
///
/// Returns [`Error::Fork`] if `fork(2)` fails, or [`Error::Wait`] if
/// `waitpid(2)` fails or the child did not exit normally (was killed by a
/// signal, for instance).
pub fn fork_and_enter<F>(child_fn: F) -> Result<i32, Error>
where
    F: FnOnce() -> i32,
{
    // SAFETY: the child performs only async-signal-safe work (a handful of
    // syscalls and one `fs::write` before ever branching into arbitrary
    // caller code) before either calling `child_fn` or exiting directly.
    match unsafe { fork() }.map_err(Error::Fork)? {
        ForkResult::Parent { child } => match waitpid(child, None).map_err(Error::Wait)? {
            WaitStatus::Exited(_, code) => Ok(code),
            other => Err(Error::ChildTerminated(other)),
        },
        ForkResult::Child => {
            let code = match enter_unprivileged_net_namespace() {
                Ok(()) => child_fn(),
                Err(_) => 111,
            };
            std::process::exit(code);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    /// Proves the primitive actually does something: the child ends up
    /// mapped to uid 0 and inside a *different* network namespace than the
    /// parent, without the parent having had any special privilege.
    #[test]
    fn fork_and_enter_creates_a_new_net_namespace() {
        let parent_netns =
            fs::read_link("/proc/self/ns/net").expect("read parent /proc/self/ns/net");

        let exit_code = fork_and_enter(move || {
            let uid_is_root = getuid().is_root();

            let child_netns = match fs::read_link("/proc/self/ns/net") {
                Ok(link) => link,
                Err(_) => return 2,
            };
            let netns_changed = child_netns != parent_netns;

            if uid_is_root && netns_changed { 0 } else { 3 }
        })
        .expect("fork_and_enter should run to completion");

        assert_eq!(
            exit_code, 0,
            "child did not observe uid 0 in a distinct net namespace (exit code {exit_code})"
        );
    }
}
