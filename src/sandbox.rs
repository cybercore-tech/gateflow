//! One coherent entry point for building a sandbox, instead of a
//! growing set of `fork_and_enter*`/`fork_veth_pair` free functions.
//!
//! This is a pure API-shape consolidation — no new capability over
//! [`crate::netns`]/[`crate::chaos`]/[`crate::veth`], which
//! [`Sandbox`]/[`PairedSandbox`] are thin wrappers over. It exists
//! because those three free functions were about to become a fourth
//! (cgroups) and a fifth (paired + chaos), and fixing the combinatorial
//! shape *before* that happens is cheaper than after: `with_chaos`,
//! `with_cgroups`, `with_chaos_and_cgroups`, `paired_with_chaos`, ...
//!
//! ```rust,ignore
//! use gateflow::Sandbox;
//!
//! // Equivalent to netns::fork_and_enter(f)
//! Sandbox::new().enter(|| 0)?;
//!
//! // Equivalent to netns::fork_and_enter_with_chaos(netem, f)
//! Sandbox::new().chaos(netem).enter(|| 0)?;
//!
//! // Equivalent to veth::fork_veth_pair(a_fn, b_fn)
//! Sandbox::paired().enter(|end| 0, |end| 0)?;
//! ```
//!
//! `Sandbox::paired()` deliberately doesn't accept `.chaos(...)` yet —
//! composing loopback chaos with a paired sandbox is a real, untested
//! combination, not just a reshape of what's already proven, so it's
//! left out rather than silently allowed and unverified. Kept as
//! `PairedSandbox`, a distinct type without a `chaos` method, so this is
//! enforced at compile time, not by a doc comment nobody reads.

use nlink::netlink::tc::NetemConfig;

use crate::Error;
use crate::netns::{fork_and_enter, fork_and_enter_with_chaos};
use crate::veth::{VethEnd, fork_veth_pair};

/// Builds a single sandboxed namespace. See the [module docs](self).
#[derive(Debug, Default)]
pub struct Sandbox {
    netem: Option<NetemConfig>,
}

impl Sandbox {
    /// Starts building a single (non-paired) sandbox.
    pub fn new() -> Self {
        Self { netem: None }
    }

    /// Starts building a paired sandbox instead — see [`PairedSandbox`].
    pub fn paired() -> PairedSandbox {
        PairedSandbox
    }

    /// Applies real `tc netem` chaos (delay/loss/jitter/reordering/
    /// corruption/duplication) to the sandbox's own loopback. See
    /// [`crate::chaos`] for why this works on `lo` at all.
    #[must_use]
    pub fn chaos(mut self, netem: NetemConfig) -> Self {
        self.netem = Some(netem);
        self
    }

    /// Forks, enters the sandbox (with chaos applied first, if
    /// configured), and runs `f`. See [`crate::netns::fork_and_enter`] /
    /// [`crate::netns::fork_and_enter_with_chaos`] for the exact
    /// mechanism and sentinel exit codes — this delegates to one or the
    /// other depending on whether [`Sandbox::chaos`] was called.
    ///
    /// # Errors
    ///
    /// Same as the function it delegates to.
    pub fn enter<F>(self, f: F) -> Result<i32, Error>
    where
        F: FnOnce() -> i32,
    {
        match self.netem {
            None => fork_and_enter(f),
            Some(netem) => fork_and_enter_with_chaos(netem, f),
        }
    }
}

/// Builds a pair of sandboxed namespaces wired together by a real veth
/// pair. Constructed via [`Sandbox::paired`], not directly — see the
/// [module docs](self) for why this doesn't (yet) accept `.chaos(...)`.
#[derive(Debug)]
pub struct PairedSandbox;

impl PairedSandbox {
    /// Forks both sides and runs `a_fn`/`b_fn` once both veth ends are
    /// confirmed up. See [`crate::veth::fork_veth_pair`] for the exact
    /// process shape and sentinel exit codes — this delegates directly
    /// to it.
    ///
    /// # Errors
    ///
    /// Same as [`crate::veth::fork_veth_pair`].
    pub fn enter<FA, FB>(self, a_fn: FA, b_fn: FB) -> Result<(i32, i32), Error>
    where
        FA: FnOnce(VethEnd) -> i32,
        FB: FnOnce(VethEnd) -> i32,
    {
        fork_veth_pair(a_fn, b_fn)
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// Confirms `Sandbox::new().enter(f)` genuinely delegates to
    /// `fork_and_enter` rather than, say, silently doing nothing — same
    /// namespace-change check as `netns`'s own test, kept independent of
    /// it rather than trusting the wrapper by inspection alone.
    #[test]
    fn sandbox_new_enters_a_real_namespace() {
        use std::fs;

        let parent_netns =
            fs::read_link("/proc/self/ns/net").expect("read parent /proc/self/ns/net");

        let exit_code = Sandbox::new()
            .enter(move || match fs::read_link("/proc/self/ns/net") {
                Ok(link) if link != parent_netns => 0,
                _ => 2,
            })
            .expect("Sandbox::new().enter should run to completion");

        assert_eq!(
            exit_code, 0,
            "sandbox did not enter a distinct namespace (code {exit_code})"
        );
    }

    /// Confirms `.chaos(...)` genuinely reaches `fork_and_enter_with_chaos`
    /// — same real-delay measurement as `netns`'s own chaos test.
    #[test]
    fn sandbox_chaos_applies_real_kernel_delay() {
        use std::net::UdpSocket;
        use std::time::Instant;

        let netem = NetemConfig::new().delay(Duration::from_millis(200)).build();

        let exit_code = Sandbox::new()
            .chaos(netem)
            .enter(|| {
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

                if start.elapsed() >= Duration::from_millis(150) {
                    0
                } else {
                    7
                }
            })
            .expect("Sandbox::new().chaos(..).enter should run to completion");

        assert_eq!(
            exit_code, 0,
            "did not observe the expected chaos delay (code {exit_code})"
        );
    }

    /// Confirms `Sandbox::paired()` genuinely reaches `fork_veth_pair` —
    /// same real UDP ping/pong as `veth`'s own test, at a fraction of the
    /// size since the mechanism itself is already proven there.
    #[test]
    fn sandbox_paired_carries_real_udp_traffic() {
        let (a_code, b_code) = Sandbox::paired()
            .enter(
                |end: VethEnd| {
                    use std::net::UdpSocket;

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
                    let peer = (end.peer_address, 9998);
                    let mut buf = [0u8; 4];
                    for _ in 0..10 {
                        if socket.send_to(b"ping", peer).is_err() {
                            std::thread::sleep(Duration::from_millis(20));
                            continue;
                        }
                        match socket.recv_from(&mut buf) {
                            Ok((n, from))
                                if &buf[..n] == b"pong" && from.ip() == end.peer_address =>
                            {
                                return 0;
                            }
                            _ => continue,
                        }
                    }
                    4
                },
                |end: VethEnd| {
                    use std::net::UdpSocket;

                    let socket = match UdpSocket::bind((end.address, 9998)) {
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
                            Err(_) => return 5,
                        }
                    }
                },
            )
            .expect("Sandbox::paired().enter should run to completion");

        assert_eq!(a_code, 0, "side A failed (code {a_code})");
        assert_eq!(b_code, 0, "side B failed (code {b_code})");
    }
}
