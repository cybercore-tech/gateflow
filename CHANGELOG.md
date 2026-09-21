# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Unprivileged Linux user + network namespace creation (`gateflow::netns`), with a test proving both the uid mapping and the namespace change actually happen.
- `fork_and_enter` now brings the sandbox's `lo` up automatically (a fresh netns's loopback starts down; previously nothing exercised this, so loopback sockets would have silently failed).
- `gateflow::chaos` + `fork_and_enter_with_chaos`: real `tc qdisc netem` (delay/loss/jitter/reordering/corruption/duplication) applied to the sandbox's own loopback via [`nlink`](https://crates.io/crates/nlink), verified with a test that measures actual elapsed delay on a real UDP round trip, not just that the call didn't error.
- `gateflow::veth` + `fork_veth_pair`: real veth-pair connectivity between two sandboxed namespaces sharing one owning user namespace (no host `CAP_NET_ADMIN`), verified with a real UDP ping/pong across the link between two otherwise fully isolated namespaces. `fork_veth_pair` guarantees both ends are confirmed up (an explicit handshake, not a fixed delay) before either closure runs — a veth end has no carrier until both sides are administratively up, and without this barrier a real caller could hit an intermittent `ENETUNREACH` depending on scheduling.
- `#[gateflow::isolated_net]` procedural macro (`gateflow-macros` crate, `macros` feature) wrapping a test function to run inside a freshly created namespace via `fork(2)`.
- Chaos parameters on `#[gateflow::isolated_net(...)]`: delay, jitter, loss, reordering, corruption, and duplication now configure the same real loopback `tc netem` path as `Sandbox::chaos(..)`.
- Opt-in hardening on `Sandbox`: cgroup v2 memory/process/CPU limits and a seccomp-BPF defense-in-depth profile that blocks namespace and selected kernel-control operations before the test body runs.
- `//-NOTES`/`DOCS`/`FIX`/`STYLE`/`RISK` dev-note comments, extracted by `build.rs` into a local-only mdBook site under `docs/` (gitignored, never rustdoc, never runs for a downstream dependent's build).
- `gateflow::Sandbox` / `PairedSandbox` (`src/sandbox.rs`): one composable entry point (`Sandbox::new().chaos(..).enter(..)`, `Sandbox::paired().enter(..)`) replacing the three separately-named `fork_*` functions, added before cgroups introduced a fourth dimension and the naming combinations started multiplying. Pure API-shape change, no new capability — thin wrappers over the same `netns`/`chaos`/`veth` functions, which stay public for direct use. `#[gateflow::isolated_net]` now expands through `Sandbox::new().enter(..)` too, so the macro and manual code share one path instead of two.
- A real integration test (`tests/isolated_net.rs`, gated on the `macros` feature via `required-features`) that actually compiles and runs `#[gateflow::isolated_net]` — previously the macro had only ever been checked at the generated-token-string level, never actually executed.
- `VethEnd::signal_done`/`wait_for_peer(timeout)`: a real cross-process completion signal for `PairedSandbox`, backed by two more handshake pipes (one per direction) created up front by `fork_veth_pair` alongside the existing setup pipes. `a_fn`/`b_fn` run in separate forked processes with no shared memory, so without this a caller has no way to know when the peer side has actually finished its work, only to guess with a fixed sleep. Found to be a real gap, not a hypothetical one, by dogfooding `gateflow` on GhostPort — its test had to work around the missing signal with a fixed 3-second sleep on the server side. Verified with a test proving genuine synchronization (elapsed time reflects when the peer actually signaled, not a race) and a separate test proving genuine timeout behavior when the peer never signals.
- Workspace scaffold: dual MIT/Apache-2.0 licensing, CI (quality/test/package/MSRV jobs), and `scripts/release-gates` (quick/full/core/macros modes).

### Changed

- MSRV raised 1.85 → 1.88: `nlink` 0.15.1 declares `rust-version = "1.85"` but its source actually needs let-chains and `is_multiple_of` (stable since 1.88) — found by the MSRV gate actually failing, not assumed.

### Fixed

- README's Installation section incorrectly claimed `gateflow` was taken on crates.io — a leftover from the `enclave` → `gateflow` rename's global find-and-replace (the claim was true of `enclave`, not `gateflow`, which is confirmed available).
