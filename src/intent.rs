//! Heuristic commit-intent classifier (keyword + diff-shape).
//!
//! Deliberately rule-based and offline: a `tv` run must never require a network
//! call. This is the seam where a learned classifier plugs in later to sharpen
//! the prose-message cases the keywords miss — but the heuristic stays as the
//! zero-dependency default.

use crate::model::Intent;

const REFACTOR: &[&str] = &[
    "retire",
    "drop",
    "delete",
    "remove",
    "dismantl",
    "rip ",
    "rip:",
    "trim",
    "prune",
    "cleanup",
    "clean up",
    "refactor",
    "rename",
    "consolidat",
    "simplif",
    "restructure",
    "sweep",
    "tombstone",
    "dropped",
    "retiring",
];
const FIX: &[&str] = &[
    "fix",
    "bug",
    "hotfix",
    "resolve",
    "correct",
    "regression",
    "broken",
    "clear",
];
const DOCS: &[&str] = &["doc", "readme", "changelog"];
const TEST: &[&str] = &["test", "pytest", "fixture", "cassette", "coverage"];
const OPS: &[&str] = &[
    "ci ",
    "ci:",
    "mise",
    "docker",
    "deploy",
    "lint",
    "ruff",
    "typecheck",
    "hook",
    "workflow",
    "pipeline",
    "release",
    "bump",
];
const FEATURE: &[&str] = &[
    "add",
    "new ",
    "introduce",
    "expose",
    "wire",
    "implement",
    "support",
    "enable",
    "ship",
    "create",
    "build",
];

fn has(s: &str, kws: &[&str]) -> bool {
    kws.iter().any(|k| s.contains(k))
}

fn frac(files: &[String], pred: impl Fn(&str) -> bool) -> f64 {
    if files.is_empty() {
        return 0.0;
    }
    let hit = files.iter().filter(|p| pred(p.as_str())).count();
    hit as f64 / files.len() as f64
}

pub fn classify(subject: &str, files: &[String], added: i64, deleted: i64) -> Intent {
    let s = subject.to_lowercase();

    if s.contains("revert") {
        return Intent::Revert;
    }
    if has(&s, FIX) {
        return Intent::Fix;
    }
    // Diff-shape: a strongly subtractive change is cleanup regardless of wording.
    let subtractive = deleted > 2 * added.max(1) && (added + deleted) > 15;
    if has(&s, REFACTOR) || subtractive {
        return Intent::Refactor;
    }
    if frac(files, |p| p.ends_with(".md") || p.starts_with("docs/")) > 0.6 || has(&s, DOCS) {
        return Intent::Docs;
    }
    if frac(files, |p| p.contains("tests/") || p.starts_with("test")) > 0.6 {
        return Intent::Test;
    }
    if has(&s, OPS) {
        return Intent::Ops;
    }
    if frac(files, |p| {
        p.starts_with("static/")
            || p.starts_with("templates/")
            || p.ends_with(".css")
            || p.ends_with(".js")
            || p.ends_with(".html")
    }) > 0.6
    {
        return Intent::Web;
    }
    if has(&s, FEATURE) {
        return Intent::Feature;
    }
    Intent::Other
}
