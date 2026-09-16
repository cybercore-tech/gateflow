# gateflow

[![CI](https://github.com/darkstardevx/gateflow/actions/workflows/ci.yml/badge.svg)](https://github.com/darkstardevx/gateflow/actions/workflows/ci.yml)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

**gateflow isolates test code inside a real, unprivileged Linux network namespace — not a simulated one.**

Crates like [`turmoil`](https://docs.rs/turmoil) and [`madsim`](https://docs.rs/madsim) get you fast, deterministic network tests by *simulating* the network in userspace. `gateflow` takes the opposite trade: real kernel network namespaces, real sockets, real `tc netem` chaos — slower, but nothing is faked. If a test passes here, it passed against the same networking stack production actually runs on.

> ⚠️ **Early and incomplete.** Namespace creation and real `tc netem` chaos on the sandbox's own loopback both work and are tested. Interface wiring (veth pairs, for chaos *between* sandboxed namespaces) and resource limits (cgroups v2) are not built yet — see [Roadmap](#roadmap). Treat this as a learning project in progress, not a released tool.

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

Not published yet. For now, depend on it by path or git:

```toml
[dependencies]
gateflow = { git = "https://github.com/darkstardevx/gateflow", features = ["macros"] }
```

## Quick Start

```rust,ignore
#[gateflow::isolated_net]
fn sees_its_own_namespace() {
    // Runs as uid 0 inside a network namespace nothing else on the host
    // can see — no host root required to get here.
}
```

Under the hood, `#[gateflow::isolated_net]` expands to a `#[test]` that forks the process and, in the child — which `unshare(2)` requires to be single-threaded, which a normal `cargo test` binary is not — enters a fresh user + network namespace, brings its `lo` up, and runs your test body. See [`gateflow::netns`](src/netns.rs) for the primitive directly, if you want to drive it yourself instead of through the macro.

### Real kernel chaos, not simulated

`netem` is an *egress* qdisc — loopback traffic still leaves through `lo` on its way back to itself, so a root `netem` qdisc on the sandbox's `lo` really does delay/drop/reorder ordinary `127.0.0.1` traffic:

```rust,ignore
use gateflow::chaos::{NetemConfig, Percent};
use gateflow::netns::fork_and_enter_with_chaos;
use std::time::Duration;

let netem = NetemConfig::new()
    .delay(Duration::from_millis(100))
    .loss(Percent::new(1.0))
    .build();

fork_and_enter_with_chaos(netem, || {
    // sockets bound to 127.0.0.1 in here see real ~100ms delay and
    // ~1% loss, enforced by the kernel — not simulated.
    0
})?;
```

Not yet wired into the `#[gateflow::isolated_net]` macro (no attribute syntax for chaos parameters yet) — drive `fork_and_enter_with_chaos` directly for now.

## Architecture

```text
┌─────────────────────────┐
│  #[gateflow::isolated_net]│   proc-macro (crates/gateflow-macros)
└────────────┬─────────────┘
             │ expands to
             ▼
┌─────────────────────────┐
│   fork(2)                │   parent waits; child is guaranteed
├─────────────────────────┤   single-threaded right after fork
│   unshare(CLONE_NEWUSER  │
│           | CLONE_NEWNET)│   src/netns.rs — the one real primitive
├─────────────────────────┤
│   uid_map / gid_map /    │   maps caller to uid 0, gid 0 —
│   setgroups=deny         │   inside the new namespace only
├─────────────────────────┤
│   lo up (+ netem, if     │   real tc qdisc via nlink —
│   with_chaos was used)   │   src/chaos.rs
├─────────────────────────┤
│   your test body runs    │
└─────────────────────────┘
```

## Roadmap

Deliberately narrow right now, on purpose — the predecessor design this grew out of tried to build routing, congestion control, telemetry, and chaos injection all before anything compiled. Not repeating that:

- [x] Unprivileged user + network namespace creation (`src/netns.rs`), `lo` brought up automatically
- [x] `#[gateflow::isolated_net]` test-attribute macro
- [x] Real chaos via `tc qdisc netem` on the sandbox's own loopback (`src/chaos.rs`, `fork_and_enter_with_chaos`) — loss / latency / jitter / reordering / corruption / duplication, real kernel enforcement, verified against actual measured delay
- [ ] veth pair wiring via netlink (no shelling out to `ip`) — needed for chaos *between* two sandboxed namespaces, not just within one
- [ ] Chaos parameters on the `#[gateflow::isolated_net]` macro itself (currently `fork_and_enter_with_chaos` only)
- [ ] cgroups v2 resource limits per test
- [ ] Optional seccomp-bpf profile per sandboxed test
- [ ] Publish to crates.io (name confirmed available as `gateflow`/`gateflow-macros`, not yet registered)

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT) at your option.
