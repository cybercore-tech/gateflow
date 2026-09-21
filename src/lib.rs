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
//! Early and incomplete. [`Sandbox`] is the entry point: a single
//! unprivileged network namespace with a working loopback, optional real
//! `tc netem` chaos on it ([`Sandbox::chaos`]), or (via
//! [`Sandbox::paired`]) two namespaces wired together by a real veth
//! pair. The `#[gateflow::isolated_net]` attribute macro (`macros`
//! feature, see the `gateflow-macros` companion crate) wraps a test to
//! run inside a plain (non-chaos, non-paired) sandbox. Resource limits
//! Optional cgroup v2 resource limits and a defense-in-depth seccomp-BPF
//! profile are available through [`CgroupLimits`] and [`SeccompProfile`].
//!
//! [`Sandbox`] is a thin builder over [`netns`]/[`chaos`]/[`veth`], which
//! stay public for direct use if you want to skip it — see their own
//! docs for the exact mechanisms and sentinel exit codes.
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

pub mod chaos;
pub mod error;
pub mod hardening;
pub mod netns;
pub mod sandbox;
pub mod veth;

pub use error::Error;
pub use hardening::{CgroupLimits, SeccompProfile};
pub use sandbox::{PairedSandbox, Sandbox};

/// Wraps a test function so its body runs inside a freshly created,
/// unprivileged network namespace instead of the host's. Requires the
/// `macros` feature. See [`gateflow_macros::isolated_net`] for the full
/// docs.
#[cfg(feature = "macros")]
pub use gateflow_macros::isolated_net;
