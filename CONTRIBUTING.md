# 🚦 Contributor & Maintainer Gate Guide

![gate](https://img.shields.io/badge/gate-non--negotiable-critical) ![rustfmt](https://img.shields.io/badge/style-rustfmt-orange) ![clippy](https://img.shields.io/badge/lint-clippy--D--warnings-blue) ![docs](https://img.shields.io/badge/docs-rustdoc--D--warnings-blueviolet)

This document defines the quality gate every change to this workspace must pass. It applies equally to the maintainer and to any contributor — there is no separate "quick PR" path.

## Table of Contents

- [Philosophy](#-philosophy)
- [Running the Gate](#-running-the-gate)
- [Step-by-Step](#-step-by-step)
- [Dev Notes (`//-TAG` comments)](#-dev-notes---tag-comments)
- [Adding a New Companion Crate](#-adding-a-new-companion-crate)
- [Release Readiness](#-release-readiness)
- [Official Docs](#-official-docs)

---

## ⚖️ Philosophy

This is a **stability-first** project — the whole reason it trades away turmoil/madsim-style in-memory simulation for real kernel namespaces is that stability matters more here than test speed. The gate exists so "it compiles on my machine" is never sufficient evidence that a change is safe to merge. Every step below is deliberately redundant with the others in some way — that redundancy is the point.

Nothing in this gate is optional and nothing is skipped "just this once." If a step is wrong for a specific situation, fix the step (in this document and `scripts/release-gates`) — don't route around it.

---

## 🏃 Running the Gate

Everything lives in one script, `scripts/release-gates`, with a mode argument:

```sh
./scripts/release-gates quick   # fmt, whitespace, check, clippy — fast, run constantly while developing
./scripts/release-gates full    # quick + tests + doctests + strict rustdoc (workspace AND per-crate) + feature matrix + MSRV
./scripts/release-gates core    # full + package verification + publish dry-run, for the `gateflow` crate
./scripts/release-gates macros  # full + package verification + publish dry-run, for `gateflow-macros`
```

**Checkpoint complete** (a feature, a fix, a PR, done): run `full` at minimum.

**Release readiness** (about to tag/publish a specific crate): run that crate's release mode (`core` or `macros`) — it re-runs `full` from scratch first, unmodified. Don't rely on a checkpoint pass from earlier in the branch's life; dependencies and sibling crates may have moved since.

CI (`.github/workflows/ci.yml`) runs the equivalent checks on every push, including on a pinned MSRV toolchain — `scripts/release-gates full` is what to run locally before pushing so CI isn't your first signal.

---

## 🔍 Step-by-Step

<details>
<summary><strong>Whitespace checks</strong> — <code>git diff --check</code> / <code>git diff --cached --check</code></summary>

**What it does:** Scans the diff (unstaged, then staged) for whitespace errors — trailing whitespace, space-before-tab, blank lines at EOF.

**When to run it:** First, always — near-instant, and catches sloppy diffs before spending time on anything that compiles fine but is cosmetically broken.

**What a failure usually means:** An editor without "trim trailing whitespace on save," or a paste from a source with different whitespace conventions.

**Fix:** The output names the file and line. Strip it and re-stage.

**Docs:** [`git-diff(1)` — the `--check` option](https://git-scm.com/docs/git-diff#Documentation/git-diff.txt---check)
</details>

<details>
<summary><strong><code>cargo fmt --all -- --check</code></strong> — formatting, workspace-wide</summary>

**What it does:** Runs `rustfmt` across every crate in the workspace, reporting whether anything *would* change without rewriting files.

**What a failure usually means:** Someone committed without running `cargo fmt`.

**Fix:** Run `cargo fmt --all` (no `-- --check`) and commit the result. Never hand-fix formatting.

**Docs:** [rustfmt](https://github.com/rust-lang/rustfmt)
</details>

<details>
<summary><strong><code>cargo check</code> / <code>cargo clippy -- -D warnings</code></strong> — compiles clean, lints clean</summary>

**What it does:** `check` type-checks every target with every feature enabled together (feature unification). `clippy -- -D warnings` repeats that plus Clippy's full lint suite, with every lint — including `warn`-level ones — promoted to a hard error.

**What a failure usually means:** A compile error, a feature-gated code path nobody exercises with default features locally, or a real Clippy lint hit.

**Fix:** Read the diagnostic. For Clippy, check the [lint index](https://rust-lang.github.io/rust-clippy/master/index.html) and either fix the pattern or add a narrowly-scoped `#[allow(clippy::lint_name)]` with a comment explaining why — never a blanket allow.

**Docs:** [`cargo check`](https://doc.rust-lang.org/cargo/commands/cargo-check.html) · [Cargo feature unification](https://doc.rust-lang.org/cargo/reference/features.html#feature-unification) · [Clippy](https://doc.rust-lang.org/clippy/)
</details>

<details>
<summary><strong><code>cargo test</code> (unit/integration + <code>--doc</code>)</strong> — the actual test suite</summary>

**What it does:** Runs every unit/integration test, then separately every doctest (the ```` ```rust ```` blocks inside `///` comments, each compiled as its own tiny crate).

**What a failure usually means:** A real regression, a feature-interaction bug that only appears with `--all-features`, or — for doctests specifically — a doc example that fell out of sync with the real API.

**Fix:** Standard debugging for the former. For the latter, update the example, or mark it ```` ```rust,no_run ```` deliberately if it's illustrative rather than runnable.

**Docs:** [`cargo test`](https://doc.rust-lang.org/cargo/commands/cargo-test.html) · [Documentation tests](https://doc.rust-lang.org/rustdoc/write-documentation/documentation-tests.html)
</details>

<details>
<summary><strong>Strict rustdoc — workspace AND crate-local</strong> — <code>RUSTDOCFLAGS="-D warnings" cargo doc ... --no-deps</code></summary>

**What it does:** Builds the actual docs, with rustdoc's own warnings (broken intra-doc links, malformed doc-comment markup) promoted to hard errors.

**Why both scopes:** The workspace-wide pass (`--workspace`) catches cross-crate breakage — e.g. `gateflow-macros` linking to something in `gateflow` that got renamed. The crate-local pass (`-p <crate>`) catches what workspace feature unification can mask: a doc/feature bug that only appears when a crate is resolved on its own, which is exactly what a downstream `cargo add gateflow` does. **Both are required, not either/or.**

**What a failure usually means:** A broken intra-doc link (a renamed/moved item — this bit `gateflow-macros` once already, linking to `gateflow::netns::fork_and_enter` from a crate that doesn't depend on `gateflow`), or invalid doc-comment markup.

**Fix:** Fix the link or markup. Don't disable the lint to unblock a PR.

**Docs:** [rustdoc](https://doc.rust-lang.org/rustdoc/index.html) · [`RUSTDOCFLAGS`](https://doc.rust-lang.org/cargo/reference/environment-variables.html) · [Lint levels](https://doc.rust-lang.org/rustc/lints/levels.html)
</details>

<details>
<summary><strong>Root feature matrix</strong></summary>

**What it does:** Checks the `gateflow` crate with each feature enabled *alone* (`--no-default-features --features <one>`), not just all-together.

**What a failure usually means:** A feature that silently depends on another feature being enabled too, only caught because `--all-features` normally masks it.

**Fix:** Either declare the real dependency in `Cargo.toml`'s `[features]` table, or make the feature stand alone.
</details>

<details>
<summary><strong>Package contents / publish dry-run</strong> — release modes only</summary>

**What it does:** `cargo package --list` shows exactly what would ship in the `.crate` archive; `cargo publish --dry-run` verifies it actually packages and would be accepted by crates.io, without publishing.

**When to run it:** Only as part of `scripts/release-gates core` / `macros` — this is the last check before an actual `cargo publish`.

**Docs:** [`cargo package`](https://doc.rust-lang.org/cargo/commands/cargo-package.html) · [`cargo publish`](https://doc.rust-lang.org/cargo/commands/cargo-publish.html)
</details>

<details>
<summary><strong>MSRV check</strong> — pinned toolchain, full gate re-run</summary>

**What it does:** Re-runs check/test/doctest/strict-rustdoc on the exact Rust version declared as `rust-version` in `Cargo.toml` (currently 1.88.0), not just whatever's installed locally.

**What a failure usually means:** Code using a language/std feature newer than the declared MSRV, or a dependency whose *default* version resolution picked something incompatible (see the MSRV-pin comments already in `Cargo.toml` for examples of this from the sibling `diagprint` project).

**Fix:** Either avoid the newer feature, or bump `rust-version` deliberately (that's a real decision, not a silent fix — say so in the PR).

**Docs:** [MSRV in the Cargo reference](https://doc.rust-lang.org/cargo/reference/manifest.html#the-rust-version-field)
</details>

---

## 📝 Dev Notes (`//-TAG` comments)

Same block-marker convention as the `cyberdeck` project, adapted. A tagged comment:

```rust
//-NOTES: Short title
// Prose content, plain `//` comments, can span multiple lines.
//-END
```

Tags, and what each means:

| Tag | Use it for |
|---|---|
| `NOTES` | General context/rationale worth surfacing beyond an inline comment — including AI-collaboration context and developer-operational notes; those were split into their own `AI`/`DEV` tags briefly and folded back in, the boundary wasn't earning its keep |
| `DOCS` | A reminder that this needs real `///` rustdoc written eventually |
| `FIX` | A known defect or gap |
| `STYLE` | A stylistic convention worth staying consistent with |
| `RISK` | A way something can go wrong if changed carelessly — security/isolation-boundary reasoning is the main case here, but not the only one |

`build.rs` extracts every tagged block into a local mdBook site at `docs/` on every `cargo build`/`check`/`test` — **maintainer-only, never committed** (`docs/` is gitignored) and **never part of rustdoc** (plain `//` comments never appear there regardless, but the aggregated view is a much more discoverable artifact than scattered source comments, so it stays fully local on top of that). Browse it with `mdbook serve docs` (loopback only) or `mdbook build docs` + open `docs/book/index.html`.

**The bar for tagging something: would you want to jump straight back here later?** A real bug found and fixed, a load-bearing constraint that isn't obvious from the code alone, a design decision deliberately deferred — not routine implementation detail, and not a plain "should do this eventually" (that's an ordinary `// TODO` comment, not a `//-DOCS`/`//-FIX` block). This is a judgment call, not something a gate can check — no lint can verify "important enough," only that a tag is well-formed. Keep the tag count low enough that the generated site stays worth skimming; if you're tagging most of what you write, the bar has slipped.

The extraction only runs inside gateflow's own git checkout (checked via a `.git` directory next to `Cargo.toml`, which `cargo package` always excludes) — a `cargo add gateflow` downstream consumer's build never triggers it.

**Don't nest tagged blocks inside each other.** There's no `CODE`-style tag here specifically because cyberdeck's version has a real bug with that: each tag's regex independently scans for the *next* `//-END` it finds, so a tag opened inside another tag's block steals the outer block's closing marker and silently truncates it. Gateflow's tags only wrap prose, never functional code spans, which avoids the precondition — keep it that way rather than reintroducing a `CODE` tag later without also fixing the underlying extraction to be nesting-aware.

---

## 📦 Adding a New Companion Crate

1. `cargo new --lib crates/gateflow-<name>` and add it to the root `Cargo.toml`'s `[workspace] members`.
2. Add its package name to the `packages` array near the top of `scripts/release-gates`.
3. If it's release-worthy on its own (not just an internal helper), add a `run_<name>_release` function mirroring `run_macros_release`, and a case in the final `case "$mode"` dispatch.
4. Add it to the `for package in gateflow gateflow-macros; do` loops in `.github/workflows/ci.yml` (crate-local strict rustdoc, package contents).
5. Give it its own `README.md` with a Quality Gate section pointing at `../../scripts/release-gates` and `../../CONTRIBUTING.md`, matching `crates/gateflow-macros/README.md`.

---

## 🏁 Release Readiness

```sh
./scripts/release-gates core     # release the gateflow crate
./scripts/release-gates macros   # release the gateflow-macros crate
```

Both require a clean working tree (`require_clean_tree` in the script) and re-run the full gate before ever touching `cargo package`/`cargo publish --dry-run` — "it passed at checkpoint" is not evidence it passes today.

---

## 📚 Official Docs

- [The Cargo Book](https://doc.rust-lang.org/cargo/)
- [The rustc Book — Lints](https://doc.rust-lang.org/rustc/lints/index.html)
- [The rustdoc Book](https://doc.rust-lang.org/rustdoc/index.html)
- [Clippy](https://doc.rust-lang.org/clippy/)
- [rustfmt](https://github.com/rust-lang/rustfmt)
- [`git-diff(1)`](https://git-scm.com/docs/git-diff)
- [GitHub Actions: Building and testing Rust](https://docs.github.com/en/actions/use-cases-and-examples/building-and-testing/building-and-testing-rust)
