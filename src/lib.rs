//! `gateflow` isolates test code inside a real, unprivileged Linux network
//! namespace — not a simulated one.
//!
//! Crates like [`turmoil`](https://docs.rs/turmoil) and
//! [`madsim`](https://docs.rs/madsim) get you fast, deterministic tests by
//! *simulating* the network in userspace. `gateflow` takes the opposite
//! trade: real kernel network namespaces, real sockets, real `tc netem`
//! chaos — slower, but nothing is faked. If a test passes here, it passed
//! against the same networking stack production runs on.
//!
//! # Status
//!
//! Early and incomplete. [`netns`] provides the working primitive:
//! creating an unprivileged network namespace, and the `#[gateflow::isolated_net]`
//! attribute macro (`macros` feature, see the `gateflow-macros` companion
//! crate) wraps a test to run inside one. Interface wiring (veth pairs),
//! resource limits (cgroups v2), and traffic-shaping chaos (`tc netem`)
//! are not built yet.
//!
//! # Platform
//!
//! Linux only. Namespace isolation is a Linux kernel feature with no
//! portable equivalent.

#![warn(missing_docs)]

#[cfg(not(target_os = "linux"))]
compile_error!(
    "gateflow only supports Linux (namespace isolation is a Linux-specific kernel feature)"
);

pub mod error;
pub mod netns;

pub use error::Error;

/// Wraps a test function so its body runs inside a freshly created,
/// unprivileged network namespace instead of the host's. Requires the
/// `macros` feature. See [`gateflow_macros::isolated_net`] for the full
/// docs.
#[cfg(feature = "macros")]
pub use gateflow_macros::isolated_net;
