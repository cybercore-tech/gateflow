# gateflow

[![CI](https://github.com/darkstardevx/gateflow/actions/workflows/ci.yml/badge.svg)](https://github.com/darkstardevx/gateflow/actions/workflows/ci.yml)
[![Project site](https://img.shields.io/badge/project%20site-GitHub%20Pages-8b7cff.svg)](https://darkstardevx.github.io/gateflow/)
[![Crates.io](https://img.shields.io/crates/v/gateflow.svg)](https://crates.io/crates/gateflow)
[![Discord](https://img.shields.io/discord/1229923929959960616?logo=discord&color=%237289da)](https://discord.gg/2WvtfwQjVc)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**gateflow isolates test code inside a real, unprivileged Linux network namespace — not a simulated one.**

Crates like [`turmoil`](https://docs.rs/turmoil) and [`madsim`](https://docs.rs/madsim) get you fast, deterministic network tests by *simulating* the network in userspace. `gateflow` takes the opposite trade: real kernel network namespaces, real sockets, real `tc netem` chaos — slower, but nothing is faked. If a test passes here, it passed against the same networking stack production actually runs on.

> **v0.1.0 is published.** Namespace creation, real `tc netem` chaos on the sandbox's own loopback, real veth-pair connectivity *between* two sandboxed namespaces, opt-in cgroup v2 limits, and an opt-in seccomp-BPF defense-in-depth profile are implemented. The hardening controls require an appropriately delegated host, and the remaining scope is tracked in the [Roadmap](#roadmap).

> ⚠️ **Early development.** gateflow is new and may still break or behave unexpectedly across kernels, distributions, CI runners, and delegated cgroup setups. Please [file an issue](https://github.com/darkstardevx/gateflow/issues) with your reproduction details, or [join the Discord](https://discord.gg/2WvtfwQjVc) to compare notes with the project community.

## Why not just simulate it?

Because a simulation is a model of the kernel's behavior, and models can be wrong in ways that only show up in production. `gateflow`'s bet is that for network-adjacent code where *stability matters more than test speed*, running against the real kernel networking stack — inside real isolation so tests don't interfere with each other or the host — is worth being slower.

## Platform

Linux only. Namespace isolation is a Linux kernel feature with no portable equivalent — the crate fails to compile on anything else, loudly, rather than silently doing nothing.

**Ubuntu 24.04 and newer** (this includes GitHub's `ubuntu-latest` runners — confirmed by CI failing on this exact issue) ship `kernel.apparmor_restrict_unprivileged_userns=1` by default, which blocks `CLONE_NEWUSER` outright. If `enter_unprivileged_net_namespace` fails with something like `write failed /proc/self/uid_map: Operation not permitted`, that's this. Fix:

```sh
sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0
```

(or load an AppArmor profile permitting it for your binary specifically, if a blanket sysctl isn't acceptable in your environment).

## Installation

For the Rust library:

```toml
[dependencies]
gateflow = { version = "0.1.0", features = ["macros"] }
```

The companion diagnostic CLI is also available from crates.io:

```sh
cargo install gateflow --version 0.1.0
gateflow doctor
```

For supported Linux hosts, install `cosign` first, then use the signed binary installer pinned to the release tag:

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://github.com/darkstardevx/gateflow/raw/v0.1.0/scripts/install.sh | sh
```

The installer requires `cosign`, `curl`, `tar`, and `sha256sum`; it verifies the Sigstore-signed checksum manifest and then verifies the selected binary before installing it to `~/.local/bin`. Set `GATEFLOW_VERSION` to install another tagged release or `GATEFLOW_INSTALL_DIR` to choose a different destination.

## Quick Start

[`Sandbox`](src/sandbox.rs) is the entry point:

```rust,ignore
use gateflow::Sandbox;

Sandbox::new().enter(|| {
    // Runs as uid 0 inside a network namespace nothing else on the host
    // can see — no host root required to get here.
    0
})?;
```

Or, as sugar for exactly that (no chaos, no pairing), the attribute macro:

```rust,ignore
#[gateflow::isolated_net]
fn sees_its_own_namespace() {
    // same thing, expands to Sandbox::new().enter(..) under the hood
}
```

Under the hood, both fork the process and, in the child — which `unshare(2)` requires to be single-threaded, which a normal `cargo test` binary is not — enter a fresh user + network namespace, bring its `lo` up, and run your test body.

### Real kernel chaos, not simulated

`netem` is an *egress* qdisc — loopback traffic still leaves through `lo` on its way back to itself, so a root `netem` qdisc on the sandbox's `lo` really does delay/drop/reorder ordinary `127.0.0.1` traffic:

```rust,ignore
use gateflow::Sandbox;
use gateflow::chaos::{NetemConfig, Percent};
use std::time::Duration;

let netem = NetemConfig::new()
    .delay(Duration::from_millis(100))
    .loss(Percent::new(1.0))
    .build();

Sandbox::new().chaos(netem).enter(|| {
    // sockets bound to 127.0.0.1 in here see real ~100ms delay and
    // ~1% loss, enforced by the kernel — not simulated.
    0
})?;
```

The same settings can be attached directly to the test macro. Supported
parameters are `delay_ms`, `jitter_ms`, `loss_percent`, `reorder_percent`,
`corrupt_percent`, and `duplicate_percent`:

```rust,ignore
#[gateflow::isolated_net(delay_ms = 100, jitter_ms = 20, loss_percent = 1.0)]
fn tolerates_a_slow_lossy_loopback() {
    // Real tc netem is applied to lo before this body runs.
}
```

Durations are integer milliseconds and percentages are clamped to
`0.0..=100.0`. Reordering follows the kernel's requirement that a delay is
also configured.

### Real connectivity between two sandboxes

Two namespaced processes, wired together by a real veth pair, without host `CAP_NET_ADMIN` — the second namespace is owned by the *same* user namespace the first one created, not the host's. Both closures only run once both ends are confirmed up (a real handshake, not a fixed delay):

```rust,ignore
use gateflow::Sandbox;
use gateflow::veth::VethEnd;
use std::time::Duration;

let (a_code, b_code) = Sandbox::paired().enter(
    |end: VethEnd| {
        // end.address / end.peer_address are real, reachable only
        // through the veth link — nothing else bridges these two
        // namespaces. Do real work here, then signal it's done
        // instead of making the peer guess how long to wait.
        end.signal_done().unwrap();
        0
    },
    |end: VethEnd| {
        if !end.wait_for_peer(Duration::from_secs(5)).unwrap() {
            return 1; // A never signaled within the timeout
        }
        0
    },
)?;
```

`end.signal_done()` / `end.wait_for_peer(timeout)` are a real cross-process completion signal — `a_fn`/`b_fn` run in separate forked processes with no shared memory, so without this, a caller has no way to know when the other side has actually finished, only to guess with a fixed sleep. Found to be a real gap (not a hypothetical one) by dogfooding `gateflow` on [GhostPort](https://github.com/darkstardevx/ghostport), which originally had to work around the missing signal with a fixed 3-second sleep.

`Sandbox::paired()` doesn't accept `.chaos(..)` yet — combining loopback chaos with a paired sandbox is a real, untested combination, not just a reshape of what's already proven, so it's deliberately left out for now (enforced at compile time: `PairedSandbox` has no `chaos` method).

### Optional hardening

Resource limits and syscall hardening are explicit opt-ins on a single
sandbox:

```rust,ignore
use gateflow::{CgroupLimits, Sandbox, SeccompProfile};

Sandbox::new()
    // Point root(..) at a cgroup v2 subtree delegated to this user when
    // /sys/fs/cgroup itself is not writable.
    .cgroup_limits(CgroupLimits::new().memory_max(256 * 1024 * 1024).pids_max(64))
    .seccomp(SeccompProfile::deny_namespace_changes())
    .enter(|| {
        // The child is limited before this body runs.
        0
    })?;
```

The cgroup API currently supports memory, process-count, and CPU quota
limits. The built-in seccomp profile blocks namespace changes, mount/umount,
ptrace, BPF, reboot, and namespace-bearing `clone` calls while preserving
ordinary test threads, files, and sockets. It is defense in depth rather
than a complete syscall allow-list.

See [`gateflow::netns`](src/netns.rs) / [`gateflow::veth`](src/veth.rs) for the primitives `Sandbox` is built on — including the exact process shape (a fork of a fork, not two siblings) and the full sentinel-code tables for diagnosing a setup failure — if you want to drive them directly instead of through the builder.

## Architecture

```text
┌─────────────────────────┐
│   Sandbox::new()          │   src/sandbox.rs — one entry point over
│     .chaos(..)? .enter()  │   netns/chaos/veth, or Sandbox::paired()
└────────────┬─────────────┘   for the two-namespace veth form
             │
             ▼
┌─────────────────────────┐
│   fork(2)                │   parent waits; child is guaranteed
├─────────────────────────┤   single-threaded right after fork
│   unshare(CLONE_NEWUSER  │
│           | CLONE_NEWNET)│   src/netns.rs — the base primitive
├─────────────────────────┤
│   uid_map / gid_map /    │   maps caller to uid 0, gid 0 —
│   setgroups=deny         │   inside the new namespace only
├─────────────────────────┤
│   lo up (+ netem, if     │   real tc qdisc via nlink —
│   .chaos(..) was used)   │   src/chaos.rs
├─────────────────────────┤
│   your test body runs    │
└─────────────────────────┘
```

`#[gateflow::isolated_net]` (crates/gateflow-macros) expands to exactly `Sandbox::new().enter(..)` — the macro and manual code go through the same path, not two separate ones.

## Roadmap

Deliberately narrow right now, on purpose — the predecessor design this grew out of tried to build routing, congestion control, telemetry, and chaos injection all before anything compiled. Not repeating that:

- [x] Unprivileged user + network namespace creation (`src/netns.rs`), `lo` brought up automatically
- [x] `#[gateflow::isolated_net]` test-attribute macro
- [x] Real chaos via `tc qdisc netem` on the sandbox's own loopback (`src/chaos.rs`, `fork_and_enter_with_chaos`) — loss / latency / jitter / reordering / corruption / duplication, real kernel enforcement, verified against actual measured delay
- [x] veth pair wiring via netlink (`src/veth.rs`, `fork_veth_pair`) — real connectivity between two sandboxed namespaces under one shared user namespace, no host `CAP_NET_ADMIN`, verified with a real UDP round trip
- [x] `Sandbox`/`PairedSandbox` (`src/sandbox.rs`) — one composable entry point over the three growing `fork_*` functions, done before cgroups added a fourth dimension and the combinations multiplied; the macro now expands through it too, not a separate path
- [x] `VethEnd::signal_done`/`wait_for_peer` — a real cross-process completion signal for `PairedSandbox`, closing a gap found by dogfooding on GhostPort (its test previously had to fall back on a fixed sleep)
- [ ] Chaos parameters on the `#[gateflow::isolated_net]` macro itself (currently `Sandbox::new().chaos(..)` only)
- [ ] `tc netem` on the veth link itself, not just loopback — now that real inter-sandbox connectivity exists
- [x] Opt-in cgroups v2 memory/process/CPU limits per test (requires a delegated cgroup root)
- [x] Opt-in seccomp-bpf defense-in-depth profile per sandboxed test
- [x] Diagnostic CLI and signed Linux binary release workflow
- [x] Publish `gateflow` and `gateflow-macros` 0.1.0 to crates.io

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
