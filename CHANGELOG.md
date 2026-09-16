# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Unprivileged Linux user + network namespace creation (`gateflow::netns`), with a test proving both the uid mapping and the namespace change actually happen.
- `fork_and_enter` now brings the sandbox's `lo` up automatically (a fresh netns's loopback starts down; previously nothing exercised this, so loopback sockets would have silently failed).
- `gateflow::chaos` + `fork_and_enter_with_chaos`: real `tc qdisc netem` (delay/loss/jitter/reordering/corruption/duplication) applied to the sandbox's own loopback via [`nlink`](https://crates.io/crates/nlink), verified with a test that measures actual elapsed delay on a real UDP round trip, not just that the call didn't error.
- `gateflow::veth` + `fork_veth_pair`: real veth-pair connectivity between two sandboxed namespaces sharing one owning user namespace (no host `CAP_NET_ADMIN`), verified with a real UDP ping/pong across the link between two otherwise fully isolated namespaces.
- `#[gateflow::isolated_net]` procedural macro (`gateflow-macros` crate, `macros` feature) wrapping a test function to run inside a freshly created namespace via `fork(2)`.
- `//-NOTES`/`DOCS`/`FIX`/`STYLE`/`RISK` dev-note comments, extracted by `build.rs` into a local-only mdBook site under `docs/` (gitignored, never rustdoc, never runs for a downstream dependent's build).
- Workspace scaffold: dual MIT/Apache-2.0 licensing, CI (quality/test/package/MSRV jobs), and `scripts/release-gates` (quick/full/core/macros modes).

### Changed

- MSRV raised 1.85 → 1.88: `nlink` 0.15.1 declares `rust-version = "1.85"` but its source actually needs let-chains and `is_multiple_of` (stable since 1.88) — found by the MSRV gate actually failing, not assumed.

### Fixed

- README's Installation section incorrectly claimed `gateflow` was taken on crates.io — a leftover from the `enclave` → `gateflow` rename's global find-and-replace (the claim was true of `enclave`, not `gateflow`, which is confirmed available).
