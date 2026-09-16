# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Added

- Unprivileged Linux user + network namespace creation (`enclave::netns`), with a test proving both the uid mapping and the namespace change actually happen.
- `#[enclave::isolated_net]` procedural macro (`enclave-macros` crate, `macros` feature) wrapping a test function to run inside a freshly created namespace via `fork(2)`.
- Workspace scaffold: dual MIT/Apache-2.0 licensing, CI (quality/test/package/MSRV jobs), and `scripts/release-gates` (quick/full/core/macros modes).
