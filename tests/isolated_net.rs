//! Proves `#[gateflow::isolated_net]` actually compiles and runs as a
//! real, standalone test — not just that its generated tokens contain
//! the expected substrings (that's `gateflow-macros`'s own unit test;
//! this crate has never had an integration test that actually exercises
//! the macro until now).
//!
//! Only requires `required-features = ["macros"]` in `Cargo.toml`
//! (rather than gating the whole file with `#[cfg(feature = "macros")]`)
//! so a default-features `cargo test` skips this file cleanly instead of
//! failing to compile it.

/// The macro's expansion wraps the whole function body as a closure run
/// post-fork, so — unlike `netns`'s own direct test — there's no hook to
/// capture "before" state to compare against. What's still directly
/// checkable from inside: the uid mapping really happened, driven
/// through the macro's generated `Sandbox::new().enter(..)` call, not
/// just through calling the underlying function directly.
#[gateflow::isolated_net]
fn isolated_net_maps_to_uid_zero() {
    assert_eq!(
        nix::unistd::getuid().as_raw(),
        0,
        "macro-driven sandbox did not observe uid 0"
    );
}
