//! Extracts `//-TAG: ... //-END` dev-note comments from source into a
//! local mdBook site under `docs/` — never committed, never published,
//! not part of the crate's rustdoc. Plain `//` comments (not `///`)
//! never appear in generated API docs regardless, but the *aggregated*
//! notes page is a more discoverable artifact than scattered comments,
//! so the whole `docs/` output stays gitignored on top of that.
//!
//! Convention (same block-marker mechanism as `cyberdeck`'s `build.rs`,
//! adapted — no `CODE`-style tag, since those are prone to a real bug in
//! that implementation: a tag nested inside another tag's block steals
//! the outer block's closing `//-END`, silently truncating it. Gateflow
//! avoids the precondition entirely by only tagging prose, never
//! wrapping functional code spans):
//!
//! ```text
//! //-NOTES: Short title
//! // Prose content, plain `//` comments, can span multiple lines.
//! //-END
//! ```
//!
//! Tags: `NOTES` (context/rationale — including AI-collaboration
//! context and developer-operational notes; those were split out as
//! `AI`/`DEV` briefly and folded back in, the boundary wasn't earning
//! its keep), `DOCS` (should become real `///` docs eventually), `FIX`
//! (a known defect), `STYLE` (a stylistic convention worth staying
//! consistent with), `RISK` (a way something can go wrong if changed
//! carelessly — security/isolation-boundary reasoning is the main case
//! here, but not the only one, so this is broader than a `SEC`-only
//! bucket would be).

use std::fs;
use std::path::{Path, PathBuf};

use regex::Regex;
use walkdir::WalkDir;

struct TagRule {
    tag: &'static str,
    regex: Regex,
    target_folder: String,
}

fn to_slug(name: &str) -> String {
    let slug: String = name
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let mut out = String::with_capacity(slug.len());
    let mut last_was_underscore = false;
    for c in slug.chars() {
        if c == '_' {
            if !last_was_underscore {
                out.push(c);
            }
            last_was_underscore = true;
        } else {
            out.push(c);
            last_was_underscore = false;
        }
    }
    out.trim_matches('_').to_string()
}

/// Strips the leading `//` (and one following space, if present) from
/// each line of a captured block, so the rendered note reads as plain
/// prose instead of raw comment syntax.
fn strip_comment_prefixes(content: &str) -> String {
    content
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let stripped = trimmed.strip_prefix("//").unwrap_or(trimmed);
            stripped.strip_prefix(' ').unwrap_or(stripped)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn create_rule(tag: &'static str, folder: &str) -> TagRule {
    let pattern = format!(r"(?s)//-{tag}:\s*(.*?)\r?\n(.*?)\s*//-END");
    TagRule {
        tag,
        regex: Regex::new(&pattern).expect("dev-note tag pattern is a fixed, valid regex"),
        target_folder: folder.to_string(),
    }
}

fn main() {
    // Skip entirely unless this is a git checkout (i.e. gateflow's own
    // development tree), not a `cargo add gateflow` consumer's vendored
    // copy. `cargo package`/`cargo publish` always exclude VCS
    // directories, so a `.git` next to Cargo.toml reliably means "this
    // is the real repo, not a packaged dependency" — the standard trick
    // for this (there's no `CARGO_PRIMARY_PACKAGE`-equivalent env var
    // exposed to a build script at *runtime*; it turns out to only be
    // set while *compiling* build.rs itself, which is useless here —
    // confirmed with `cargo build -vv` rather than assumed).
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    if !Path::new(&manifest_dir).join(".git").exists() {
        return;
    }

    let rules = vec![
        create_rule("NOTES", "docs/src/notes"),
        create_rule("DOCS", "docs/src/docs-owed"),
        create_rule("FIX", "docs/src/fixes"),
        create_rule("STYLE", "docs/src/style"),
        create_rule("RISK", "docs/src/risk"),
    ];

    scaffold_book();

    for rule in &rules {
        if Path::new(&rule.target_folder).exists() {
            fs::remove_dir_all(&rule.target_folder).ok();
        }
        fs::create_dir_all(&rule.target_folder).expect("create dev-notes output folder");
    }

    // Scan every crate's src/ in the workspace, not just the root
    // package — a plain WalkDir::new("src") would miss
    // crates/gateflow-macros/src entirely.
    let scan_roots = ["src", "crates"];

    for rule in &rules {
        for root in scan_roots {
            if !Path::new(root).exists() {
                continue;
            }
            for entry in WalkDir::new(root).into_iter().filter_map(|e| e.ok()) {
                let path = entry.path();
                if path.extension().and_then(|s| s.to_str()) != Some("rs") {
                    continue;
                }
                let content = fs::read_to_string(path).unwrap_or_default();
                for cap in rule.regex.captures_iter(&content) {
                    let raw_title = cap[1].trim();
                    let body = strip_comment_prefixes(cap[2].trim());
                    let slug = to_slug(raw_title);
                    let file_path = format!("{}/{slug}.md", rule.target_folder);
                    let source = path.display();
                    let markdown = format!("## {raw_title}\n\n{body}\n\n*Source: `{source}`*\n");
                    fs::write(&file_path, markdown).expect("write dev-note snippet");
                }
            }
        }
    }

    generate_summary(&rules);

    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=crates");
}

/// Writes `docs/book.toml` and `docs/src/README.md` if they don't exist
/// yet — unlike the tag folders, these are never wiped/regenerated, so
/// hand-edits (e.g. rewriting the intro) survive rebuilds. `docs/` is
/// gitignored entirely, so a fresh clone needs this to bootstrap the
/// site from nothing on the first `cargo build`.
fn scaffold_book() {
    fs::create_dir_all("docs/src").expect("create docs/src");

    let book_toml = "docs/book.toml";
    if !Path::new(book_toml).exists() {
        fs::write(
            book_toml,
            "[book]\n\
             title = \"gateflow dev notes\"\n\
             src = \"src\"\n\
             \n\
             [build]\n\
             build-dir = \"book\"\n",
        )
        .expect("write docs/book.toml");
    }

    let readme = "docs/src/README.md";
    if !Path::new(readme).exists() {
        fs::write(
            readme,
            "# gateflow dev notes\n\n\
             Maintainer-only notes extracted from `//-NOTES`/`//-DOCS`/`//-FIX`/\
             `//-STYLE`/`//-RISK` comments in source by \
             `build.rs`. Regenerated on \
             every `cargo build`/`check`/`test` — never hand-edit the generated \
             pages, only this README and `book.toml` survive a rebuild.\n\n\
             Never committed (`docs/` is gitignored) and never part of rustdoc — \
             plain `//` comments don't appear there regardless, but the \
             aggregated view here is a more discoverable artifact than scattered \
             source comments, so it stays fully local.\n\n\
             Build: `mdbook build docs`. Browse: `mdbook serve docs` (loopback \
             only, like everything else on this box).\n",
        )
        .expect("write docs/src/README.md");
    }
}

/// Reads the title back out of a generated snippet's own `## <title>`
/// first line, instead of reconstructing it from the (already-slugified,
/// lowercased) filename — so `SUMMARY.md` shows the real title
/// (`uid/gid mapping order is load-bearing, not stylistic`), not a
/// mangled one (`uid gid mapping order is load bearing not stylistic`).
fn title_from_snippet(path: &Path) -> String {
    let content = fs::read_to_string(path).unwrap_or_default();
    content
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("## "))
        .unwrap_or("(untitled)")
        .to_string()
}

fn generate_summary(rules: &[TagRule]) {
    let summary_path = "docs/src/SUMMARY.md";
    let mut summary = String::from("# Summary\n\n- [Introduction](README.md)\n");

    let categories: Vec<(&str, &str)> = rules
        .iter()
        .map(|r| {
            let header = match r.tag {
                "NOTES" => "Notes",
                "DOCS" => "Docs Owed",
                "FIX" => "Known Fixes",
                "STYLE" => "Style Notes",
                "RISK" => "Known Risks",
                other => other,
            };
            (r.target_folder.strip_prefix("docs/src/").unwrap(), header)
        })
        .collect();

    for (dir, header) in categories {
        let dir_path = format!("docs/src/{dir}");
        if !Path::new(&dir_path).exists() {
            continue;
        }

        let mut files: Vec<PathBuf> = WalkDir::new(&dir_path)
            .max_depth(1)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().and_then(|s| s.to_str()) == Some("md"))
            .map(|e| e.path().to_path_buf())
            .collect();

        if files.is_empty() {
            continue;
        }

        summary.push_str(&format!("\n# {header}\n"));
        files.sort_by_key(|p| p.file_name().unwrap().to_str().unwrap().to_lowercase());

        for file in files {
            let relative = file.strip_prefix("docs/src/").unwrap().to_str().unwrap();
            let title = title_from_snippet(&file);
            summary.push_str(&format!("- [{title}]({relative})\n"));
        }
    }

    fs::write(summary_path, summary).expect("write dev-notes SUMMARY.md");
}
