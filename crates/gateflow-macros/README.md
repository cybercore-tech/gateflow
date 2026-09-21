# gateflow-macros

Procedural macro companion to [`gateflow`](../..) — provides
`#[gateflow::isolated_net]`. Enable the `macros` feature on the `gateflow`
crate instead of depending on this crate directly.

The attribute accepts optional loopback chaos parameters:

```rust,ignore
#[gateflow::isolated_net(delay_ms = 100, loss_percent = 1.0)]
fn network_test() {
    // Runs with real tc netem delay and packet loss on loopback.
}
```

Supported keys are `delay_ms`, `jitter_ms`, `loss_percent`,
`reorder_percent`, `corrupt_percent`, and `duplicate_percent`.

## 🚦 Quality Gate

This crate is part of the workspace gate — see [`CONTRIBUTING.md`](../../CONTRIBUTING.md).

- Crate-local (run before every commit touching this crate):
  ```sh
  ../../scripts/release-gates quick
  ```
- Full workspace (run before every PR/merge, and again before release):
  ```sh
  ../../scripts/release-gates full
  ```

## License

Licensed under either of [Apache License, Version 2.0](../../LICENSE-APACHE) or [MIT license](../../LICENSE-MIT) at your option.
