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

//-NOTES: veth pairs need a second namespace, not host CAP_NET_ADMIN
// Real inter-sandbox connectivity (two `fork_and_enter`ed processes
// actually talking to each other, not just each in isolation) can't be
// done by wiring a veth pair into the host's root netns — an
// unprivileged caller genuinely doesn't have CAP_NET_ADMIN there, and
// getting it would mean asking users for host root, which is exactly
// the promise this crate exists to avoid breaking.
//
// The version that stays unprivileged: create a *second* net namespace
// that's also owned by the same user namespace `unshare(CLONE_NEWUSER)`
// already created (repeat CLONE_NEWNET from a process already inside
// that userns), then move a veth pair's second end into it. Both ends
// stay under capabilities the caller already legitimately has. This is
// closer to a small virtual-LAN primitive than a one-function addition —
// scope it as its own deliberate piece of work, not a quick follow-up
// to the loopback chaos primitive.
//-END

use std::fs;

use nix::sched::{CloneFlags, unshare};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, fork, getgid, getuid};
use nlink::netlink::tc::NetemConfig;
use nlink::{Connection, Route};

use crate::Error;

/// Puts the *current* process into a fresh, unprivileged user + network
/// namespace.
///
/// On success, the calling process is mapped to uid 0 / gid 0 inside the
/// new namespace (and only inside it — this grants no privilege on the
/// host) and owns a network namespace containing nothing but a loopback
/// interface — **down**, like every fresh network namespace's `lo`; this
/// function does not bring it up (it has no async runtime to do that
/// with). [`fork_and_enter`] does. Wiring the namespace to anything else
/// (a veth pair, routes) is not implemented yet.
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

    //-RISK: uid/gid mapping order is load-bearing, not stylistic
    // `setgroups` must be denied *before* `gid_map` is written, or the
    // write fails with EPERM for an unprivileged process — the kernel
    // refuses to let a process claim arbitrary supplementary groups via
    // gid_map without first proving it can't `setgroups()` into them.
    // Reordering these three writes silently breaks
    // `enter_unprivileged_net_namespace` on every kernel, not just some.
    //-END
    write_proc_self("/proc/self/setgroups", "deny")?;
    write_proc_self("/proc/self/uid_map", &format!("0 {uid} 1"))?;
    write_proc_self("/proc/self/gid_map", &format!("0 {gid} 1"))?;

    Ok(())
}

fn write_proc_self(path: &'static str, contents: &str) -> Result<(), Error> {
    fs::write(path, contents).map_err(|source| Error::IdMap { path, source })
}

/// Enters a fresh network namespace *without* creating a new user
/// namespace — for a process that has already inherited unprivileged
/// root via a fork from something that called
/// [`enter_unprivileged_net_namespace`]. Two such namespaces, entered by
/// a process and its own further fork, are both owned by the same user
/// namespace — required for [`crate::veth::fork_veth_pair`] to be able
/// to move a veth pair's peer end into the second one without host
/// `CAP_NET_ADMIN`. Calling this in a process that never went through
/// `enter_unprivileged_net_namespace` (or a descendant of one) will fail
/// the same way `enter_unprivileged_net_namespace` itself does on a host
/// with unprivileged user namespaces disabled.
pub fn enter_sibling_net_namespace() -> Result<(), Error> {
    unshare(CloneFlags::CLONE_NEWNET).map_err(Error::Namespace)
}

/// Forks the current process and enters an unprivileged net namespace in
/// the child, immediately after `fork(2)` — see the [module docs](self)
/// for why the fork is necessary at all. Unlike calling
/// [`enter_unprivileged_net_namespace`] directly, this also brings the
/// new namespace's `lo` up, so ordinary loopback sockets work in
/// `child_fn` without extra setup.
///
/// `child_fn` runs inside the new namespace and its return value becomes
/// the child process's exit code. The parent blocks in `waitpid(2)` and
/// returns that exit code once the child terminates.
///
/// If entering the namespace fails, the child exits with status `111`;
/// if bringing `lo` up fails, `112` — both without ever running
/// `child_fn` (values unlikely to collide with `child_fn`'s own exit
/// codes).
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
    fork_and_enter_inner(None, child_fn)
}

/// Identical to [`fork_and_enter`], but additionally applies `netem`
/// (`tc qdisc ... netem`, real kernel delay/loss/jitter/reordering — see
/// [`crate::chaos`]) to the sandboxed namespace's own `lo` before running
/// `child_fn`. Because loopback traffic really does traverse `lo`'s
/// egress qdisc on the way back to itself, this gives `child_fn` real,
/// kernel-enforced chaos on ordinary `127.0.0.1` sockets.
///
/// If applying the netem configuration fails, the child exits with
/// status `113` without ever running `child_fn`.
///
/// # Errors
///
/// Same as [`fork_and_enter`].
pub fn fork_and_enter_with_chaos<F>(netem: NetemConfig, child_fn: F) -> Result<i32, Error>
where
    F: FnOnce() -> i32,
{
    fork_and_enter_inner(Some(netem), child_fn)
}

fn fork_and_enter_inner<F>(netem: Option<NetemConfig>, child_fn: F) -> Result<i32, Error>
where
    F: FnOnce() -> i32,
{
    // SAFETY: `fork(2)` itself is sound to call here regardless of what
    // the child does next — no invariant of `fork` depends on it. What
    // *isn't* covered by this comment (and is a real, accepted tradeoff
    // of this design, not a guarantee) is strict POSIX async-signal-safety
    // of the child's post-fork work: `enter_unprivileged_net_namespace`
    // and `run_loopback_setup` allocate (`format!`, a Tokio runtime's own
    // startup) before running `child_fn`, which is technically unsound if
    // the parent was mid-malloc on another thread at the exact moment of
    // fork. Accepted in practice for a `fork_and_enter` caller — a plain
    // `#[test]` function not itself holding an allocator lock across the
    // call — the same way most fork-then-more-than-exec code in the wild
    // does; a stricter design would fork+exec a tiny helper binary
    // instead, which is future work if this ever bites someone for real.
    match unsafe { fork() }.map_err(Error::Fork)? {
        ForkResult::Parent { child } => match waitpid(child, None).map_err(Error::Wait)? {
            WaitStatus::Exited(_, code) => Ok(code),
            other => Err(Error::ChildTerminated(other)),
        },
        ForkResult::Child => {
            let code = match enter_unprivileged_net_namespace() {
                Ok(()) => match run_loopback_setup(netem) {
                    Ok(()) => child_fn(),
                    Err(_) => 113,
                },
                Err(_) => 111,
            };
            std::process::exit(code);
        }
    }
}

/// Builds a one-shot Tokio runtime and brings `lo` up (applying `netem`
/// too, if given) inside it. A fresh runtime per call, not a shared one,
/// because this only ever runs once, immediately post-fork, in a child
/// about to either run `child_fn` or exit — there's nothing to amortize.
fn run_loopback_setup(netem: Option<NetemConfig>) -> Result<(), Error> {
    let runtime = tokio::runtime::Runtime::new().map_err(Error::Runtime)?;
    runtime.block_on(configure_loopback(netem))
}

async fn configure_loopback(netem: Option<NetemConfig>) -> Result<(), Error> {
    let conn = Connection::<Route>::new().map_err(Error::Netlink)?;
    conn.set_link_up("lo").await.map_err(Error::Netlink)?;
    if let Some(netem) = netem {
        conn.apply_netem("lo", netem)
            .await
            .map_err(Error::Netlink)?;
    }
    Ok(())
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

    /// A fresh network namespace's `lo` starts down — proves
    /// `fork_and_enter` actually brings it up, not just that the
    /// namespace exists. A UDP send-to-self would fail (`ENETUNREACH` or
    /// similar) if `lo` were still down.
    #[test]
    fn fork_and_enter_brings_loopback_up() {
        use std::net::UdpSocket;

        let exit_code = fork_and_enter(|| {
            let socket = match UdpSocket::bind("127.0.0.1:0") {
                Ok(s) => s,
                Err(_) => return 2,
            };
            let addr = match socket.local_addr() {
                Ok(a) => a,
                Err(_) => return 3,
            };
            if socket.send_to(b"ping", addr).is_err() {
                return 4;
            }
            let mut buf = [0u8; 4];
            match socket.recv_from(&mut buf) {
                Ok((n, _)) if &buf[..n] == b"ping" => 0,
                _ => 5,
            }
        })
        .expect("fork_and_enter should run to completion");

        assert_eq!(
            exit_code, 0,
            "loopback UDP send-to-self failed inside the sandbox (exit code {exit_code}) \
             — lo is probably still down"
        );
    }

    /// Proves `fork_and_enter_with_chaos` applies a *real* kernel netem
    /// qdisc, not a no-op: a UDP packet sent to self over loopback with a
    /// configured 200ms delay should take measurably close to that long
    /// to arrive, since netem's egress delay on `lo` really does apply to
    /// loopback traffic on its way back to itself.
    #[test]
    fn fork_and_enter_with_chaos_applies_real_kernel_delay() {
        use std::net::UdpSocket;
        use std::time::{Duration, Instant};

        use crate::chaos::NetemConfig;

        let netem = NetemConfig::new().delay(Duration::from_millis(200)).build();

        let exit_code = fork_and_enter_with_chaos(netem, || {
            let socket = match UdpSocket::bind("127.0.0.1:0") {
                Ok(s) => s,
                Err(_) => return 2,
            };
            let addr = match socket.local_addr() {
                Ok(a) => a,
                Err(_) => return 3,
            };
            if socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .is_err()
            {
                return 4;
            }

            let start = Instant::now();
            if socket.send_to(b"ping", addr).is_err() {
                return 5;
            }
            let mut buf = [0u8; 4];
            if socket.recv_from(&mut buf).is_err() {
                return 6;
            }
            let elapsed = start.elapsed();

            // Generous lower bound (well under the configured 200ms) to
            // absorb scheduling jitter while still failing hard if netem
            // wasn't really applied (an unaffected loopback round trip
            // is sub-millisecond, not "close to 150ms").
            if elapsed >= Duration::from_millis(150) {
                0
            } else {
                eprintln!("gateflow test: observed delay {elapsed:?}, expected >= 150ms");
                7
            }
        })
        .expect("fork_and_enter_with_chaos should run to completion");

        assert_eq!(
            exit_code, 0,
            "did not observe the expected netem-induced delay (exit code {exit_code})"
        );
    }
}
