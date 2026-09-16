//! Real veth-pair connectivity between two sandboxed namespaces — no host
//! `CAP_NET_ADMIN` required. See the `//-NOTES` block in `netns.rs` for
//! why this needs a second namespace owned by the *same* user namespace
//! rather than wiring into the host's root netns.
//!
//! # Process shape
//!
//! ```text
//! Coordinator (caller)
//!   └─ fork → A: enter_unprivileged_net_namespace() [new userns U, netns N1]
//!        └─ fork → B: enter_sibling_net_namespace()  [same U, new netns N2]
//! ```
//!
//! `B` is a fork of `A`, not a sibling forked directly by the
//! coordinator — that's what makes `N1` and `N2` share one owning user
//! namespace. Three pipes sequence the handshake so the veth pair is
//! only created once `B`'s namespace actually exists, and `B` only
//! configures its end once the pair actually exists in it:
//!
//! 1. `B` → `A`: "my namespace is ready" (so `A` can safely target `B`'s
//!    pid when creating the veth pair).
//! 2. `A` → `B`: "the pair exists and my end is configured" (so `B` can
//!    safely configure its own end).
//! 3. `A` → Coordinator: `B`'s exit code (the coordinator only directly
//!    `waitpid`s `A`; `A` reaps `B` itself and relays the result, since
//!    `B` is `A`'s child, not the coordinator's).
//!
//! A fifth and sixth pipe (per direction: one each way) back
//! [`VethEnd::signal_done`]/[`VethEnd::wait_for_peer`] — `a_fn`/`b_fn`
//! run in the same two forked processes with no shared memory, so
//! without an explicit signal, neither side has any way to know when
//! the other has finished its own work. Found to be a real, not
//! hypothetical, gap by using this crate on real code (see darknotes'
//! GhostPort note): a caller without it has to fall back on a fixed
//! sleep, guessing how long the peer's work usually takes instead of
//! knowing when it's actually done.
//!
//! All pipes are created once, up front, by the coordinator, so every
//! descendant inherits every fd it needs through *both* forks — each
//! process then closes the ends it doesn't use itself.

use std::net::Ipv4Addr;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use nix::fcntl::{FcntlArg, OFlag, fcntl};
use nix::sys::wait::{WaitStatus, waitpid};
use nix::unistd::{ForkResult, Pid, fork, pipe, read, write};
use nlink::netlink::link::VethLink;
use nlink::netlink::{Connection, Route};

use crate::Error;
use crate::netns::{enter_sibling_net_namespace, enter_unprivileged_net_namespace};

/// A `/24` private range used only for this one point-to-point link —
/// each `fork_veth_pair` call gets its own pair of fresh namespaces, so
/// there's no collision risk between separate calls reusing the same
/// addresses.
const SUBNET_PREFIX: u8 = 24;
const ADDR_A: Ipv4Addr = Ipv4Addr::new(10, 200, 100, 1);
const ADDR_B: Ipv4Addr = Ipv4Addr::new(10, 200, 100, 2);
const IFACE_A: &str = "veth-a";
const IFACE_B: &str = "veth-b";

/// Everything one side of a veth-connected namespace pair needs to use
/// its end: the interface name (already up, address already assigned),
/// both addresses, and a way to synchronize with the peer side — the
/// two closures given to [`fork_veth_pair`] run in separate forked
/// processes with no shared memory, so without this there is no way for
/// one side to know when the other has finished (or reached some point
/// in) its own work.
///
/// Not `Clone` — the completion-signal pipe ends are unique per process,
/// same reason [`std::fs::File`] isn't `Clone` either.
#[derive(Debug)]
pub struct VethEnd {
    /// The local interface name (`veth-a` or `veth-b`), already up.
    pub interface: &'static str,
    /// This side's address on the point-to-point link.
    pub address: Ipv4Addr,
    /// The peer's address — what to actually connect/send to.
    pub peer_address: Ipv4Addr,
    my_done_w: OwnedFd,
    peer_done_r: OwnedFd,
}

impl VethEnd {
    /// Signals to the peer side that this side has finished its work.
    /// Idempotent to call at most once per side in practice (a second
    /// call still just writes a byte nobody's necessarily still reading
    /// for) — there's no protocol here beyond "at least one signal
    /// happened."
    ///
    /// # Errors
    ///
    /// Returns [`Error::Pipe`] if the underlying write fails — this
    /// would mean the peer process is already gone (its read end
    /// closed), not a transient condition worth retrying.
    pub fn signal_done(&self) -> Result<(), Error> {
        write(&self.my_done_w, &[1u8]).map_err(Error::Pipe)?;
        Ok(())
    }

    /// Blocks until the peer side calls [`VethEnd::signal_done`], or
    /// `timeout` elapses. A timeout returns `Ok(false)`, not an error —
    /// it's an ordinary, expected outcome for a caller polling/retrying
    /// on its own terms, not a failure of the mechanism itself.
    ///
    /// # Errors
    ///
    /// Returns [`Error::Pipe`] if the underlying read fails for a reason
    /// other than "no data yet" (a genuine OS-level error, not a normal
    /// timeout).
    pub fn wait_for_peer(&self, timeout: Duration) -> Result<bool, Error> {
        let deadline = Instant::now() + timeout;
        let mut buf = [0u8; 1];
        loop {
            match read(&self.peer_done_r, &mut buf) {
                Ok(1) => return Ok(true),
                // EOF: the peer exited without ever signaling. Not a
                // pipe error — there's simply never going to be a
                // signal now, so this is a normal "no" rather than
                // something to retry until the timeout.
                Ok(_) => return Ok(false),
                Err(nix::Error::EAGAIN) => {
                    if Instant::now() >= deadline {
                        return Ok(false);
                    }
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(Error::Pipe(e)),
            }
        }
    }
}

/// Marks a pipe's read end non-blocking so [`VethEnd::wait_for_peer`]
/// can poll it with a real deadline instead of blocking forever on a
/// peer that might already be gone.
fn set_nonblocking(fd: &OwnedFd) -> Result<(), Error> {
    let flags = fcntl(fd, FcntlArg::F_GETFL).map_err(Error::Pipe)?;
    let mut oflags = OFlag::from_bits_truncate(flags);
    oflags.insert(OFlag::O_NONBLOCK);
    fcntl(fd, FcntlArg::F_SETFL(oflags)).map_err(Error::Pipe)?;
    Ok(())
}

/// The pipe fds one side keeps past its own fork point, bundled so
/// they're threaded through function signatures as one thing instead of
/// three easily-mixed-up `OwnedFd`s.
struct SideAPipes {
    b_ready_r: OwnedFd,
    wire_done_w: OwnedFd,
    b_end_up_r: OwnedFd,
    b_result_w: OwnedFd,
    /// A's end of the completion-signal pipes, handed to `a_fn` inside
    /// its `VethEnd` — not used by `run_side_a` itself.
    a_done_w: OwnedFd,
    b_done_r: OwnedFd,
}

struct SideBPipes {
    b_ready_w: OwnedFd,
    wire_done_r: OwnedFd,
    b_end_up_w: OwnedFd,
    /// B's end of the completion-signal pipes, handed to `b_fn` inside
    /// its `VethEnd` — not used by `run_side_b` itself.
    b_done_w: OwnedFd,
    a_done_r: OwnedFd,
}

/// Forks two namespaces wired together by a real veth pair (see the
/// [module docs](self) for the exact process shape) and runs `a_fn` in
/// the first, `b_fn` in the second, once both ends are configured and
/// up. Both namespaces have a working `lo` too, same as
/// [`crate::netns::fork_and_enter`].
///
/// Returns `(a_fn`'s exit code, `b_fn`'s exit code`)`. A setup failure on
/// either side is reported as one of these sentinel codes instead of
/// running the corresponding closure at all — a real closure return
/// value could coincide with one, so treat any of these exact codes as
/// "setup failed here," not as your closure's own result, if you see
/// one unexpectedly:
///
/// | Code | Side | Meaning |
/// |---|---|---|
/// | 121 | A | `enter_unprivileged_net_namespace` failed |
/// | 122 | A | forking B failed |
/// | 123 | A | never heard B's "namespace ready" signal (B died first) |
/// | 124 | A | creating/configuring the veth pair failed |
/// | 125 | A | signaling B that wiring is done failed |
/// | 126 | A | `waitpid` on B failed, or B didn't exit normally |
/// | 127 | A | relaying B's exit code back to the coordinator failed |
/// | 128 | A | never heard B confirm its own end is up (B died first) |
/// | 131 | B | `enter_sibling_net_namespace` failed |
/// | 132 | B | signaling "namespace ready" to A failed |
/// | 133 | B | never heard A's "wiring done" signal (A died first) |
/// | 134 | B | configuring its own veth end failed |
/// | 135 | B | signaling "my end is up" back to A failed |
///
/// `-1` for `b_fn`'s code specifically means `A` itself never got far
/// enough to report one (e.g. `A`'s own namespace entry failed before it
/// could even fork `B`, so `B` never ran and never had anything to
/// report).
///
/// # Errors
///
/// Returns [`Error::Fork`]/[`Error::Wait`]/[`Error::Pipe`] if the
/// coordination machinery itself (not the sandboxed setup inside it)
/// fails.
pub fn fork_veth_pair<FA, FB>(a_fn: FA, b_fn: FB) -> Result<(i32, i32), Error>
where
    FA: FnOnce(VethEnd) -> i32,
    FB: FnOnce(VethEnd) -> i32,
{
    let (b_ready_r, b_ready_w) = pipe().map_err(Error::Pipe)?;
    let (wire_done_r, wire_done_w) = pipe().map_err(Error::Pipe)?;
    let (b_end_up_r, b_end_up_w) = pipe().map_err(Error::Pipe)?;
    let (b_result_r, b_result_w) = pipe().map_err(Error::Pipe)?;
    let (a_done_r, a_done_w) = pipe().map_err(Error::Pipe)?;
    let (b_done_r, b_done_w) = pipe().map_err(Error::Pipe)?;

    // SAFETY: see the equivalent comment in
    // netns.rs::fork_and_enter_inner — the same accepted tradeoff
    // applies here, doubled (two forks in the descendant chain).
    match unsafe { fork() }.map_err(Error::Fork)? {
        ForkResult::Parent { child: a_pid } => {
            // Coordinator uses none of the handshake pipes directly,
            // only the result pipe's read end.
            drop(b_ready_r);
            drop(b_ready_w);
            drop(wire_done_r);
            drop(wire_done_w);
            drop(b_end_up_r);
            drop(b_end_up_w);
            drop(b_result_w);
            drop(a_done_r);
            drop(a_done_w);
            drop(b_done_r);
            drop(b_done_w);

            let a_code = match waitpid(a_pid, None).map_err(Error::Wait)? {
                WaitStatus::Exited(_, code) => code,
                other => return Err(Error::ChildTerminated(other)),
            };

            let mut buf = [0u8; 1];
            let b_code = match read(&b_result_r, &mut buf) {
                Ok(1) => i32::from(buf[0]),
                // A exited before ever writing a result (failed before
                // reaching the point where it forks B) — not a pipe
                // error, just nothing to report.
                _ => -1,
            };

            Ok((a_code, b_code))
        }
        ForkResult::Child => {
            // A: doesn't need the coordinator's exclusive read end.
            drop(b_result_r);
            let code = run_side_a(
                SideAPipes {
                    b_ready_r,
                    wire_done_w,
                    b_end_up_r,
                    b_result_w,
                    a_done_w,
                    b_done_r,
                },
                SideBPipes {
                    b_ready_w,
                    wire_done_r,
                    b_end_up_w,
                    b_done_w,
                    a_done_r,
                },
                a_fn,
                b_fn,
            );
            std::process::exit(code);
        }
    }
}

/// Runs entirely inside the forked "A" process. `b_pipes` is only ever
/// used to hand off to B at fork time — A itself never reads or writes
/// through them, it just has to keep them open until that fork happens
/// so B inherits working copies.
fn run_side_a<FA, FB>(a_pipes: SideAPipes, b_pipes: SideBPipes, a_fn: FA, b_fn: FB) -> i32
where
    FA: FnOnce(VethEnd) -> i32,
    FB: FnOnce(VethEnd) -> i32,
{
    if enter_unprivileged_net_namespace().is_err() {
        return 121;
    }

    // SAFETY: same accepted tradeoff as the outer fork — A is still
    // single-threaded here (no Tokio runtime built yet).
    let b_pid = match unsafe { fork() } {
        Ok(ForkResult::Child) => {
            // B: doesn't need A's exclusive fds.
            drop(a_pipes.wire_done_w);
            drop(a_pipes.b_end_up_r);
            drop(a_pipes.b_result_w);
            drop(a_pipes.a_done_w);
            drop(a_pipes.b_done_r);
            let code = run_side_b(a_pipes.b_ready_r, b_pipes, b_fn);
            std::process::exit(code);
        }
        Ok(ForkResult::Parent { child }) => child,
        Err(_) => return 122,
    };

    // A: doesn't need B's exclusive fds now that B has its own copies.
    drop(b_pipes.b_ready_w);
    drop(b_pipes.wire_done_r);
    drop(b_pipes.b_end_up_w);
    drop(b_pipes.b_done_w);
    drop(b_pipes.a_done_r);

    let mut ready_buf = [0u8; 1];
    match read(&a_pipes.b_ready_r, &mut ready_buf) {
        Ok(1) => {}
        _ => return 123,
    }

    let end_a = match configure_side_a(b_pid, a_pipes.a_done_w, a_pipes.b_done_r) {
        Ok(end) => end,
        Err(_) => return 124,
    };

    if write(&a_pipes.wire_done_w, &[1u8]).is_err() {
        return 125;
    }

    //-RISK: a_fn must not run until B confirms its own end is up
    // A veth end has no carrier (state DOWN, route "linkdown") until
    // *both* ends are administratively up — confirmed by reproducing it
    // by hand with plain `ip`/`unshare` before trusting it in Rust. A
    // sendto() through a linkdown route fails synchronously with
    // ENETUNREACH. Without this barrier, a_fn starts as soon as A's own
    // end is up, racing B's own (independent, concurrent) setup — real
    // callers would see intermittent ENETUNREACH depending on scheduling,
    // not just this crate's own tests. Waiting for B's explicit
    // confirmation here, not just a fixed delay, makes it a real barrier
    // instead of a hopeful guess at how long B usually takes.
    //-END
    let mut end_up_buf = [0u8; 1];
    match read(&a_pipes.b_end_up_r, &mut end_up_buf) {
        Ok(1) => {}
        _ => return 128,
    }

    //-RISK: a_fn must run before waitpid(B), not after
    // B is a separate, already-running process by this point. Reaping it
    // first (waitpid(b_pid) before a_fn) deadlocks the whole test in
    // practice, not in theory: a_fn is usually the side sending B
    // something B is blocked waiting to receive, so waiting for B to
    // exit first just blocks here while B blocks waiting on a_fn, which
    // hasn't run yet. This was a real bug during development, not a
    // hypothetical one — B timed out waiting for a ping that could never
    // arrive, exited, tore its namespace (and veth-b) down with it, and
    // only then did a_fn finally run against a peer that no longer
    // existed. A refactor that reorders these two calls back the wrong
    // way will reintroduce this exact failure, silently, since both
    // orderings compile and only one hangs/fails at runtime.
    //-END
    let a_result = a_fn(end_a);

    let b_exit = match waitpid(b_pid, None) {
        Ok(WaitStatus::Exited(_, code)) => code,
        _ => return 126,
    };

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let b_byte = b_exit.clamp(0, 255) as u8;
    if write(&a_pipes.b_result_w, &[b_byte]).is_err() {
        return 127;
    }

    a_result
}

/// Runs entirely inside the forked "B" process (itself a fork of A, not
/// of the coordinator). `b_ready_r` is A's copy of the readiness pipe —
/// unused by B, kept only so it stays alive for A's sake until B closes
/// it below (B has its own `b_ready_w` in `pipes` to actually signal on).
fn run_side_b<FB>(a_b_ready_r: OwnedFd, pipes: SideBPipes, b_fn: FB) -> i32
where
    FB: FnOnce(VethEnd) -> i32,
{
    drop(a_b_ready_r);

    if enter_sibling_net_namespace().is_err() {
        return 131;
    }

    if write(&pipes.b_ready_w, &[1u8]).is_err() {
        return 132;
    }

    let mut done_buf = [0u8; 1];
    match read(&pipes.wire_done_r, &mut done_buf) {
        Ok(1) => {}
        _ => return 133,
    }

    let end_b = match configure_side_b(pipes.b_done_w, pipes.a_done_r) {
        Ok(end) => end,
        Err(_) => return 134,
    };

    if write(&pipes.b_end_up_w, &[1u8]).is_err() {
        return 135;
    }

    b_fn(end_b)
}

/// Builds a one-shot Tokio runtime (same reasoning as
/// `netns::run_loopback_setup`: this runs once, immediately post-fork,
/// nothing to amortize) and, inside it: creates the veth pair with the
/// peer end landing directly in `peer_pid`'s namespace, assigns and
/// brings up this side's end, and brings up `lo`.
fn configure_side_a(
    peer_pid: Pid,
    my_done_w: OwnedFd,
    peer_done_r: OwnedFd,
) -> Result<VethEnd, Error> {
    set_nonblocking(&peer_done_r)?;

    let runtime = tokio::runtime::Runtime::new().map_err(Error::Runtime)?;
    runtime.block_on(async {
        let conn = Connection::<Route>::new().map_err(Error::Netlink)?;

        #[allow(clippy::cast_sign_loss)]
        let peer_pid_raw = peer_pid.as_raw() as u32;
        let veth = VethLink::new(IFACE_A, IFACE_B).peer_netns_pid(peer_pid_raw);
        conn.add_link(veth).await.map_err(Error::Netlink)?;

        conn.add_address(nlink::netlink::addr::Ipv4Address::new(
            IFACE_A,
            ADDR_A,
            SUBNET_PREFIX,
        ))
        .await
        .map_err(Error::Netlink)?;
        conn.set_link_up(IFACE_A).await.map_err(Error::Netlink)?;
        conn.set_link_up("lo").await.map_err(Error::Netlink)?;

        Ok(VethEnd {
            interface: IFACE_A,
            address: ADDR_A,
            peer_address: ADDR_B,
            my_done_w,
            peer_done_r,
        })
    })
}

/// Same shape as [`configure_side_a`] but for B's side: `veth-b` already
/// exists in this namespace (A created it there directly via
/// `peer_netns_pid`), so B only assigns its address and brings things up.
fn configure_side_b(my_done_w: OwnedFd, peer_done_r: OwnedFd) -> Result<VethEnd, Error> {
    set_nonblocking(&peer_done_r)?;

    let runtime = tokio::runtime::Runtime::new().map_err(Error::Runtime)?;
    runtime.block_on(async {
        let conn = Connection::<Route>::new().map_err(Error::Netlink)?;

        conn.add_address(nlink::netlink::addr::Ipv4Address::new(
            IFACE_B,
            ADDR_B,
            SUBNET_PREFIX,
        ))
        .await
        .map_err(Error::Netlink)?;
        conn.set_link_up(IFACE_B).await.map_err(Error::Netlink)?;
        conn.set_link_up("lo").await.map_err(Error::Netlink)?;

        Ok(VethEnd {
            interface: IFACE_B,
            address: ADDR_B,
            peer_address: ADDR_A,
            my_done_w,
            peer_done_r,
        })
    })
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
    use std::time::{Duration, Instant};

    use super::*;

    const PORT: u16 = 9999;

    /// Proves real cross-namespace connectivity, not just that setup
    /// didn't error: A and B are otherwise completely isolated network
    /// namespaces (separate `unshare`s, no shared interfaces besides the
    /// veth pair) — the only way this ping/pong can complete is if the
    /// veth pair genuinely carries traffic between them.
    ///
    /// `fork_veth_pair` itself now guarantees both ends are confirmed up
    /// before either closure runs (see the `//-RISK` note on the
    /// end-up handshake in `run_side_a`), so this loop's small retry
    /// budget is ordinary network-test hygiene (first-packet ARP
    /// resolution, scheduling jitter) — not, as an earlier version of
    /// this test needed, working around a real carrier race that the
    /// library left for every caller to hit themselves.
    #[test]
    fn fork_veth_pair_carries_real_udp_traffic() {
        let (a_code, b_code) = fork_veth_pair(
            |end: VethEnd| {
                let socket = match UdpSocket::bind((end.address, 0)) {
                    Ok(s) => s,
                    Err(_) => return 2,
                };
                if socket
                    .set_read_timeout(Some(Duration::from_millis(200)))
                    .is_err()
                {
                    return 3;
                }

                let peer = (end.peer_address, PORT);
                let mut buf = [0u8; 4];
                for _ in 0..10 {
                    if socket.send_to(b"ping", peer).is_err() {
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    match socket.recv_from(&mut buf) {
                        Ok((n, from)) if &buf[..n] == b"pong" && from.ip() == end.peer_address => {
                            return 0;
                        }
                        _ => continue,
                    }
                }
                5
            },
            |end: VethEnd| {
                let socket = match UdpSocket::bind((end.address, PORT)) {
                    Ok(s) => s,
                    Err(_) => return 2,
                };
                if socket
                    .set_read_timeout(Some(Duration::from_secs(6)))
                    .is_err()
                {
                    return 3;
                }

                let mut buf = [0u8; 4];
                loop {
                    match socket.recv_from(&mut buf) {
                        Ok((n, from)) if &buf[..n] == b"ping" => {
                            return if socket.send_to(b"pong", from).is_ok() {
                                0
                            } else {
                                4
                            };
                        }
                        Ok(_) => continue,
                        Err(e) => {
                            eprintln!("gateflow test: B never received a ping: {e}");
                            return 5;
                        }
                    }
                }
            },
        )
        .expect("fork_veth_pair should run to completion");

        eprintln!("gateflow test: a_code={a_code} b_code={b_code}");

        assert_eq!(
            a_code, 0,
            "side A did not complete the ping/pong (code {a_code})"
        );
        assert_eq!(
            b_code, 0,
            "side B did not complete the ping/pong (code {b_code})"
        );
    }

    /// Proves `signal_done`/`wait_for_peer` genuinely synchronize, not
    /// just that they return successfully: A deliberately delays before
    /// signaling, B measures how long its wait actually took. If
    /// `wait_for_peer` were a no-op that just returned `true`
    /// immediately, this would fail the elapsed-time assertion even
    /// though the boolean result would look correct.
    #[test]
    fn veth_end_signal_done_and_wait_for_peer_synchronizes() {
        let (a_code, b_code) = fork_veth_pair(
            |end: VethEnd| {
                std::thread::sleep(Duration::from_millis(300));
                if end.signal_done().is_err() {
                    return 2;
                }
                0
            },
            |end: VethEnd| {
                let start = Instant::now();
                let signaled = match end.wait_for_peer(Duration::from_secs(2)) {
                    Ok(s) => s,
                    Err(_) => return 3,
                };
                let elapsed = start.elapsed();

                if !signaled {
                    return 4;
                }
                // Generous lower bound under A's real 300ms delay, to
                // absorb scheduling jitter while still failing hard if
                // wait_for_peer returned near-instantly regardless of
                // when A actually signaled.
                if elapsed < Duration::from_millis(200) {
                    eprintln!(
                        "gateflow test: wait_for_peer returned after only {elapsed:?}, expected close to 300ms"
                    );
                    return 5;
                }
                0
            },
        )
        .expect("fork_veth_pair should run to completion");

        assert_eq!(a_code, 0, "side A failed (code {a_code})");
        assert_eq!(b_code, 0, "side B failed (code {b_code})");
    }

    /// Proves `wait_for_peer` actually enforces its timeout instead of
    /// hanging or returning instantly: A deliberately holds its end of
    /// the pipe open (without signaling) for longer than B's timeout, so
    /// B has to hit real `EAGAIN`-and-retry cycles until its own
    /// deadline, not an immediate EOF short-circuit from A exiting
    /// early.
    #[test]
    fn veth_end_wait_for_peer_times_out_if_peer_never_signals() {
        let (a_code, b_code) = fork_veth_pair(
            |_end: VethEnd| {
                // Deliberately never calls signal_done — and stays
                // alive well past B's timeout so B can't shortcut via
                // EOF instead of a real timeout.
                std::thread::sleep(Duration::from_secs(1));
                0
            },
            |end: VethEnd| {
                let start = Instant::now();
                let signaled = match end.wait_for_peer(Duration::from_millis(300)) {
                    Ok(s) => s,
                    Err(_) => return 2,
                };
                let elapsed = start.elapsed();

                if signaled {
                    return 3;
                }
                if elapsed < Duration::from_millis(250) || elapsed > Duration::from_millis(700) {
                    eprintln!(
                        "gateflow test: wait_for_peer took {elapsed:?}, expected close to 300ms"
                    );
                    return 4;
                }
                0
            },
        )
        .expect("fork_veth_pair should run to completion");

        assert_eq!(a_code, 0, "side A failed (code {a_code})");
        assert_eq!(b_code, 0, "side B failed (code {b_code})");
    }
}
