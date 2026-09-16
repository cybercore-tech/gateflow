//! Real kernel packet chaos (`tc qdisc ... netem`) for a sandboxed test's
//! own loopback interface.
//!
//! Re-exports [`nlink`]'s typed `netem` builder directly rather than
//! wrapping it — there is no `gateflow`-specific behavior to add on top
//! of "build a [`NetemConfig`], hand it to
//! [`crate::netns::fork_and_enter_with_chaos`]". A wrapper type here
//! would just be a second name for the same thing.
//!
//! # Example
//!
//! ```no_run
//! use gateflow::chaos::{NetemConfig, Percent};
//! use gateflow::netns::fork_and_enter_with_chaos;
//! use std::time::Duration;
//!
//! let netem = NetemConfig::new()
//!     .delay(Duration::from_millis(100))
//!     .loss(Percent::new(1.0))
//!     .build();
//!
//! fork_and_enter_with_chaos(netem, || {
//!     // sockets bound to 127.0.0.1 in here see real ~100ms delay and
//!     // ~1% loss, enforced by the kernel's own netem qdisc — not
//!     // simulated.
//!     0
//! })?;
//! # Ok::<(), gateflow::Error>(())
//! ```
//!
//! # Why this works on loopback
//!
//! `netem` is an *egress* qdisc: it applies to packets as they leave an
//! interface. Loopback traffic still leaves through `lo` on its way back
//! to itself, so a root `netem` qdisc on `lo` really does delay/drop/
//! reorder ordinary `127.0.0.1` traffic — this is the same trick used
//! outside Rust entirely (`tc qdisc add dev lo root netem delay 100ms`)
//! to test local-latency sensitivity without touching a real network.

pub use nlink::Percent;
pub use nlink::netlink::tc::NetemConfig;
