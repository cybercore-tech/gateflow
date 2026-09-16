# enclave-macros

Procedural macro companion to [`enclave`](../..) — provides `#[enclave::isolated_net]`. Not meant to be depended on directly; enable the `macros` feature on the `enclave` crate instead.

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
