//! Git plumbing — shells out to the `git` binary (no libgit2 dependency).
//!
//! Two passes:
//!   - `load_commits` — fast, one `git log --numstat`; powers batch/cadence/net.
//!   - `collect_cached` — the blame-at-death pass for survival-weighted
//!     flow/thrash/excision. Slow on first run (per-line blame), so it's cached
//!     keyed by the anchor sha (HEAD by default, or the `--at` commit) and runs
//!     automatically — no flag, no separate command.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::intent;
use crate::model::{Commit, Intent, Me};

const REC: char = '\u{1e}'; // ASCII record separator
const FLD: char = '\u{1f}'; // ASCII unit separator

const BINARY_EXT: &[&str] = &[
    ".png", ".jpg", ".jpeg", ".gif", ".ico", ".webp", ".avif", ".svg", ".woff", ".woff2", ".ttf",
    ".otf", ".eot", ".pdf", ".npz", ".npy", ".zip", ".gz", ".mp4", ".webm", ".mp3", ".wasm",
];

fn git(repo: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map_err(|e| format!("failed to run git (is it installed?): {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn current_branch(repo: &str) -> Result<String, String> {
    Ok(git(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string())
}

// --- anchor primitives (`--at`): resolve a rev/date to a concrete commit -------

/// The current HEAD commit sha.
pub fn head_sha(repo: &str) -> Result<String, String> {
    Ok(git(repo, &["rev-parse", "HEAD"])?.trim().to_string())
}

/// Resolve a revision (`sha`, tag, branch, `HEAD~5`, …) to a commit sha, or `Err`
/// if it isn't a commit in this repo. `--quiet` makes the failure a non-zero exit.
pub fn resolve_rev(repo: &str, rev: &str) -> Result<String, String> {
    let spec = format!("{rev}^{{commit}}");
    let out = git(repo, &["rev-parse", "--verify", "--quiet", &spec])?;
    let sha = out.trim();
    if sha.is_empty() {
        Err(format!("`{rev}` is not a commit in this repo"))
    } else {
        Ok(sha.to_string())
    }
}

/// Committer timestamp (unix) of a rev.
pub fn commit_ts(repo: &str, rev: &str) -> Result<i64, String> {
    git(repo, &["show", "-s", "--format=%ct", rev])?
        .trim()
        .parse()
        .map_err(|_| format!("no commit timestamp for `{rev}`"))
}

/// Short committer date (YYYY-MM-DD) of a rev, for display.
pub fn commit_date(repo: &str, rev: &str) -> Result<String, String> {
    Ok(git(repo, &["show", "-s", "--format=%cs", rev])?
        .trim()
        .to_string())
}

/// Newest first-parent commit on/before a date expression (`2026-03-01`,
/// `@1700000000`, `3 weeks ago`, …). `None` if the repo has nothing that old.
pub fn snap_before(repo: &str, before: &str) -> Result<Option<String>, String> {
    let arg = format!("--before={before}");
    let out = git(repo, &["rev-list", "-1", "--first-parent", &arg, "HEAD"])?;
    let sha = out.trim();
    Ok(if sha.is_empty() {
        None
    } else {
        Some(sha.to_string())
    })
}

/// All non-merge commits reachable from `anchor` (a rev — `"HEAD"` or a sha) with
/// per-file numstat, newest first. Fast. The anchor is how `--at` rewinds history.
pub fn load_commits(repo: &str, anchor: &str) -> Result<Vec<Commit>, String> {
    let fmt = format!("--pretty=format:{REC}%H{FLD}%ct{FLD}%ae{FLD}%an{FLD}%s");
    let out = git(
        repo,
        &[
            "log",
            "--no-merges",
            "--date-order",
            "--numstat",
            fmt.as_str(),
            anchor,
        ],
    )?;

    let mut commits = Vec::new();
    for rec in out.split(REC) {
        let rec = rec.trim_matches('\n');
        if rec.is_empty() {
            continue;
        }
        let mut lines = rec.split('\n');
        let header = lines.next().unwrap_or("");
        let mut h = header.split(FLD);
        let sha = h.next().unwrap_or("").to_string();
        let ts: i64 = h.next().unwrap_or("0").trim().parse().unwrap_or(0);
        let author_email = h.next().unwrap_or("").to_string();
        let author_name = h.next().unwrap_or("").to_string();
        let subject = h.next().unwrap_or("").to_string();

        let mut added = 0_i64;
        let mut deleted = 0_i64;
        let mut files = Vec::new();
        for row in lines {
            if row.trim().is_empty() {
                continue;
            }
            let mut cols = row.split('\t');
            let a = cols.next().unwrap_or("-");
            let d = cols.next().unwrap_or("-");
            let path = cols.next().unwrap_or("");
            if a != "-" {
                added += a.trim().parse::<i64>().unwrap_or(0);
            }
            if d != "-" {
                deleted += d.trim().parse::<i64>().unwrap_or(0);
            }
            if !path.is_empty() {
                files.push(path.to_string());
            }
        }

        let intent = intent::classify(&subject, &files, added, deleted);
        commits.push(Commit {
            sha,
            ts,
            subject,
            author_email,
            author_name,
            added,
            deleted,
            files,
            intent,
        });
    }
    Ok(commits)
}

// ---------------------------------------------------------------------------
// Blame-at-death collection: per-line lifetimes for the survival metrics.
// ---------------------------------------------------------------------------

/// One line-death: its age (commit clock + wall clock) and how rewrite-like the
/// killing edit was (1 = in-place rewrite/thrash, 0 = wholesale excision).
/// `kill_a`/`intro_a` index `Collection::authors` (-1 = unknown) — `kill_a` is who
/// deleted the line (the `--me` thrash lens), `intro_a` who wrote it (survival lens).
pub struct DeathRecord {
    pub age_c: f64,
    pub age_t: f64,
    pub rw: f64,
    pub kill_ts: i64,
    pub path: String,
    pub kill_a: i32,
    pub intro_a: i32,
    /// Intent of the commit that did the deleting — lets thrash be qualified by the
    /// kind of work doing the rewriting (refactor sweep vs bug-fix churn).
    pub kill_intent: Intent,
}

/// Everything the survival metrics need. `cens_*` are survivors alive at HEAD;
/// `cens_a` is each survivor's introducing author (index into `authors`, -1 unknown).
/// One per repo — never merged across repos, so each repo's KM curve stays its
/// own (repo frailty differs; a pooled curve would misweight every repo).
pub struct Collection {
    pub head: String,
    pub deaths: Vec<DeathRecord>,
    pub cens_c: Vec<f64>,
    pub cens_t: Vec<f64>,
    pub cens_a: Vec<i32>,
    pub authors: Vec<(String, String)>, // (email lowercased, name) by index
}

impl Collection {
    /// Mask over `authors`: true where the author is the running user (`--me`).
    pub fn author_mask(&self, me: &Me) -> Vec<bool> {
        self.authors
            .iter()
            .map(|(email, name)| me.matches(email, name))
            .collect()
    }
}

/// The running user's git identity for `--me`, from `git config`. None if unset.
pub fn whoami(repo: &str) -> Option<Me> {
    let email = git(repo, &["config", "user.email"])
        .ok()?
        .trim()
        .to_string();
    let name = git(repo, &["config", "user.name"])
        .unwrap_or_default()
        .trim()
        .to_string();
    if email.is_empty() && name.is_empty() {
        return None;
    }
    Some(Me { email, name })
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn is_binary(p: &str) -> bool {
    let lp = p.to_lowercase();
    BINARY_EXT.iter().any(|e| lp.ends_with(e))
}

// Data/docs/config — excluded from the hotspot complexity proxy so it points at
// code (nested JSON/YAML would otherwise dominate the indentation score).
const NONCODE_EXT: &[&str] = &[
    ".json", ".jsonl", ".ndjson", ".yaml", ".yml", ".toml", ".lock", ".csv", ".tsv", ".md", ".rst",
    ".txt", ".cfg", ".ini", ".sql",
];

fn is_noncode(p: &str) -> bool {
    let lp = p.to_lowercase();
    NONCODE_EXT.iter().any(|e| lp.ends_with(e))
}

/// `@@ -a,b +c,d @@` -> (a, b, d). Missing length defaults to 1.
fn parse_hunk(line: &str) -> Option<(i64, i64, i64)> {
    let rest = line.strip_prefix("@@ ")?;
    let mut parts = rest.split(' ');
    let minus = parts.next()?.strip_prefix('-')?;
    let plus = parts.next()?.strip_prefix('+')?;
    let side = |s: &str| -> (i64, i64) {
        if let Some((x, y)) = s.split_once(',') {
            (x.parse().unwrap_or(0), y.parse().unwrap_or(1))
        } else {
            (s.parse().unwrap_or(0), 1)
        }
    };
    let (a, b) = side(minus);
    let (_c, d) = side(plus);
    Some((a, b, d))
}

/// Per path: (parent-side deleted line numbers, added-line count). Whitespace
/// ignored so pure reformatting doesn't register as deletions.
fn file_churn(repo: &str, sha: &str) -> HashMap<String, (Vec<i64>, i64)> {
    let out = match git(repo, &["show", "-U0", "-w", "--no-color", "--format=", sha]) {
        Ok(o) => o,
        Err(_) => return HashMap::new(),
    };
    let mut map: HashMap<String, (Vec<i64>, i64)> = HashMap::new();
    let mut src: Option<String> = None;
    for line in out.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            src = if rest == "/dev/null" {
                None
            } else {
                Some(rest.strip_prefix("a/").unwrap_or(rest).to_string())
            };
        } else if line.starts_with("@@") {
            if let (Some(s), Some((a, b, d))) = (src.as_ref(), parse_hunk(line)) {
                let entry = map.entry(s.clone()).or_default();
                if b > 0 {
                    entry.0.extend(a..a + b);
                }
                entry.1 += d;
            }
        }
    }
    map
}

/// final_lineno -> (introducing sha, committer ts), parsed from blame porcelain.
fn blame_at(repo: &str, rev: &str, path: &str) -> HashMap<i64, (String, i64)> {
    let out = match git(repo, &["blame", "-p", "-w", rev, "--", path]) {
        Ok(o) => o,
        Err(_) => return HashMap::new(),
    };
    let mut line_sha: HashMap<i64, String> = HashMap::new();
    let mut sha_ts: HashMap<String, i64> = HashMap::new();
    let mut cur = String::new();
    for line in out.lines() {
        if line.starts_with('\t') {
            continue; // content line
        }
        let mut toks = line.split_whitespace();
        let Some(first) = toks.next() else { continue };
        let sha = first.strip_prefix('^').unwrap_or(first);
        if is_sha(sha) {
            // header: <sha> <orig_lineno> <final_lineno> [<num>]
            let _orig = toks.next();
            if let Some(final_s) = toks.next() {
                if let Ok(final_ln) = final_s.parse::<i64>() {
                    cur = sha.to_string();
                    line_sha.insert(final_ln, cur.clone());
                }
            }
        } else if first == "committer-time" {
            if let Some(Ok(ts)) = toks.next().map(str::parse::<i64>) {
                if !cur.is_empty() {
                    sha_ts.insert(cur.clone(), ts);
                }
            }
        }
    }
    let mut result = HashMap::with_capacity(line_sha.len());
    for (ln, sha) in line_sha {
        if let Some(&ts) = sha_ts.get(&sha) {
            result.insert(ln, (sha, ts));
        }
    }
    result
}

/// sha -> topological index over commits reachable from `anchor`, plus the
/// anchor's own index (the censoring point) and its committer ts.
fn commit_index(repo: &str, anchor: &str) -> Result<(HashMap<String, i64>, i64, i64), String> {
    let out = git(repo, &["rev-list", "--reverse", "--topo-order", anchor])?;
    let mut map = HashMap::new();
    let mut i = 0_i64;
    for sha in out.split_whitespace() {
        map.insert(sha.to_string(), i);
        i += 1;
    }
    let anchor_ts = commit_ts(repo, anchor)?;
    Ok((map, i - 1, anchor_ts))
}

/// `anchor` is the as-of rev (`"HEAD"` or a sha); `head_sha` is its resolved sha,
/// stored as the cache key. Survivors are the lines alive *at the anchor*, blamed
/// against the anchor's tree — so `--at` rewinds the censoring point cleanly.
fn collect(
    repo: &str,
    commits: &[Commit],
    anchor: &str,
    head_sha: &str,
) -> Result<Collection, String> {
    eprintln!("tv: analyzing build history (first run for this anchor — caching for next time)…");
    let (index, head_idx, head_ts) = commit_index(repo, anchor)?;
    let total = commits.len();

    // Author table: intern commit authors so deaths/survivors can be attributed
    // by index (compact in the cache). sha -> author index for the introducers.
    let mut authors: Vec<(String, String)> = Vec::new();
    let mut email_idx: HashMap<String, i32> = HashMap::new();
    let mut sha_aidx: HashMap<&str, i32> = HashMap::new();
    for c in commits {
        let el = c.author_email.to_lowercase();
        let idx = *email_idx.entry(el.clone()).or_insert_with(|| {
            authors.push((el.clone(), c.author_name.clone()));
            (authors.len() - 1) as i32
        });
        sha_aidx.insert(c.sha.as_str(), idx);
    }
    let intro_of = |isha: &str| -> i32 { sha_aidx.get(isha).copied().unwrap_or(-1) };

    let mut deaths = Vec::new();
    for (k, c) in commits.iter().enumerate() {
        if k % 50 == 0 {
            eprint!("\r  deaths {k}/{total}");
            let _ = std::io::stderr().flush();
        }
        if c.deleted == 0 {
            continue;
        }
        let Some(&kill_idx) = index.get(&c.sha) else {
            continue;
        };
        let kill_a = sha_aidx.get(c.sha.as_str()).copied().unwrap_or(-1);
        let parent = format!("{}^", c.sha);
        for (path, (dels, adds)) in file_churn(repo, &c.sha) {
            if dels.is_empty() {
                continue;
            }
            let rw = adds.min(dels.len() as i64) as f64 / dels.len() as f64;
            let bmap = blame_at(repo, parent.as_str(), path.as_str());
            for ln in dels {
                let Some((isha, its)) = bmap.get(&ln) else {
                    continue;
                };
                let Some(&iidx) = index.get(isha) else {
                    continue;
                };
                let age_c = (kill_idx - iidx) as f64;
                let age_t = (c.ts - *its) as f64 / 86400.0;
                if age_c >= 0.0 && age_t >= 0.0 {
                    deaths.push(DeathRecord {
                        age_c,
                        age_t,
                        rw,
                        kill_ts: c.ts,
                        path: path.clone(),
                        kill_a,
                        intro_a: intro_of(isha),
                        kill_intent: c.intent,
                    });
                }
            }
        }
    }
    eprintln!("\r  deaths {total}/{total}   ");

    let files = git(repo, &["ls-tree", "-r", "--name-only", anchor])?;
    let flist: Vec<&str> = files
        .lines()
        .filter(|p| !p.is_empty() && !is_binary(p))
        .collect();
    let ftotal = flist.len();
    let mut cens_c = Vec::new();
    let mut cens_t = Vec::new();
    let mut cens_a = Vec::new();
    for (k, path) in flist.iter().enumerate() {
        if k % 100 == 0 {
            eprint!("\r  survivors {k}/{ftotal}");
            let _ = std::io::stderr().flush();
        }
        for (_ln, (isha, its)) in blame_at(repo, anchor, path) {
            if let Some(&iidx) = index.get(&isha) {
                cens_c.push((head_idx - iidx) as f64);
                cens_t.push((head_ts - its) as f64 / 86400.0);
                cens_a.push(intro_of(&isha));
            }
        }
    }
    eprintln!("\r  survivors {ftotal}/{ftotal}   ");

    Ok(Collection {
        head: head_sha.to_string(),
        deaths,
        cens_c,
        cens_t,
        cens_a,
        authors,
    })
}

/// Run the blame-at-death pass, reusing an anchor-keyed cache when valid. Auto-runs
/// — no separate command or flag. `anchor` is the as-of rev (`"HEAD"` or a sha); the
/// default HEAD run keeps the plain `tv-cache`, a pinned `--at` gets its own file so
/// archaeology never clobbers the everyday cache.
pub fn collect_cached(repo: &str, commits: &[Commit], anchor: &str) -> Result<Collection, String> {
    let key = git(repo, &["rev-parse", anchor])?.trim().to_string();
    // The HEAD-default run (anchor == "HEAD") is by definition HEAD — skip the
    // extra `rev-parse HEAD` it would otherwise cost on the common warm path.
    let is_head = anchor == "HEAD" || head_sha(repo).map(|h| h == key).unwrap_or(false);
    let cache = cache_file(repo, &key, is_head);
    if let Some(path) = &cache {
        if let Some(col) = load_cache(path, &key) {
            return Ok(col);
        }
    }
    let col = collect(repo, commits, anchor, &key)?;
    if let Some(path) = &cache {
        let _ = save_cache(path, &col); // best-effort; a cache miss just recomputes
    }
    Ok(col)
}

fn cache_file(repo: &str, key: &str, is_head: bool) -> Option<PathBuf> {
    let gd = git(repo, &["rev-parse", "--absolute-git-dir"]).ok()?;
    let name = if is_head {
        "tv-cache".to_string()
    } else {
        format!("tv-cache-{}", &key[..key.len().min(12)])
    };
    Some(Path::new(gd.trim()).join(name))
}

fn load_cache(path: &Path, head: &str) -> Option<Collection> {
    let data = std::fs::read_to_string(path).ok()?;
    let mut lines = data.lines();
    let cached_head = lines.next()?.strip_prefix("TVCACHE5 ")?;
    if cached_head != head {
        return None;
    }
    let mut authors = Vec::new();
    let mut deaths = Vec::new();
    let mut cens_c = Vec::new();
    let mut cens_t = Vec::new();
    let mut cens_a = Vec::new();
    for line in lines {
        let mut t = line.split('\t');
        match t.next() {
            Some("A") => authors.push((t.next()?.to_string(), t.next().unwrap_or("").to_string())),
            Some("D") => deaths.push(DeathRecord {
                age_c: t.next()?.parse().ok()?,
                age_t: t.next()?.parse().ok()?,
                rw: t.next()?.parse().ok()?,
                kill_ts: t.next()?.parse().ok()?,
                kill_a: t.next()?.parse().ok()?,
                intro_a: t.next()?.parse().ok()?,
                kill_intent: Intent::from_label(t.next()?),
                path: t.next()?.to_string(),
            }),
            Some("C") => {
                cens_c.push(t.next()?.parse().ok()?);
                cens_t.push(t.next()?.parse().ok()?);
                cens_a.push(t.next()?.parse().ok()?);
            }
            _ => {}
        }
    }
    Some(Collection {
        head: head.to_string(),
        deaths,
        cens_c,
        cens_t,
        cens_a,
        authors,
    })
}

fn save_cache(path: &Path, col: &Collection) -> std::io::Result<()> {
    let mut s = format!("TVCACHE5 {}\n", col.head);
    for (email, name) in &col.authors {
        s.push_str(&format!("A\t{email}\t{name}\n"));
    }
    for d in &col.deaths {
        s.push_str(&format!(
            "D\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            d.age_c,
            d.age_t,
            d.rw,
            d.kill_ts,
            d.kill_a,
            d.intro_a,
            d.kill_intent.label(),
            d.path
        ));
    }
    for ((c, t), a) in col.cens_c.iter().zip(&col.cens_t).zip(&col.cens_a) {
        s.push_str(&format!("C\t{c}\t{t}\t{a}\n"));
    }
    std::fs::write(path, s)
}

/// git regex metachar escape, so an email/name `--author` pattern matches literally.
fn re_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        if "\\.^$*+?()[]{}|".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// `--author=<pat>` for each identity (OR-matched by git). Used for `--me`.
fn author_args(authors: &[String]) -> Vec<String> {
    authors
        .iter()
        .filter(|a| !a.is_empty())
        .map(|a| format!("--author={}", re_escape(a)))
        .collect()
}

/// `git log <mode> --format=` over the path-stat history, with the shared windowing
/// applied: `anchor` (`"HEAD"` or a sha) caps history at the `--at` point, `since`
/// (unix ts) trails it to recent commits (None = all history), and `authors` (if
/// non-empty) restricts to those commit authors (`--me`). `mode` is `--numstat`
/// (churn) or `--name-only` (change frequency).
fn log_paths(
    repo: &str,
    anchor: &str,
    since: Option<i64>,
    authors: &[String],
    mode: &str,
) -> Result<String, String> {
    let mut args: Vec<String> = vec![
        "log".into(),
        "--no-merges".into(),
        mode.into(),
        "--format=".into(),
    ];
    if let Some(ts) = since {
        args.push(format!("--since=@{ts}"));
    }
    args.extend(author_args(authors));
    args.push(anchor.to_string());
    let argrefs: Vec<&str> = args.iter().map(String::as_str).collect();
    git(repo, &argrefs)
}

/// Churn (added+deleted) per path, over the windowed history (see [`log_paths`]).
pub fn file_churn_totals(
    repo: &str,
    anchor: &str,
    since: Option<i64>,
    authors: &[String],
) -> Result<HashMap<String, i64>, String> {
    let out = log_paths(repo, anchor, since, authors, "--numstat")?;
    let mut m: HashMap<String, i64> = HashMap::new();
    for line in out.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut c = line.split('\t');
        let a = c.next().unwrap_or("-");
        let d = c.next().unwrap_or("-");
        let path = c.next().unwrap_or("");
        if path.is_empty() {
            continue;
        }
        let add: i64 = if a == "-" { 0 } else { a.parse().unwrap_or(0) };
        let del: i64 = if d == "-" { 0 } else { d.parse().unwrap_or(0) };
        *m.entry(path.to_string()).or_insert(0) += add + del;
    }
    Ok(m)
}

/// Change frequency: how many (windowed) commits touched each path (see
/// [`log_paths`]). The standard hotspot "change" axis — "edited often" — far less
/// size-skewed than line churn.
pub fn file_change_freq(
    repo: &str,
    anchor: &str,
    since: Option<i64>,
    authors: &[String],
) -> Result<HashMap<String, i64>, String> {
    let out = log_paths(repo, anchor, since, authors, "--name-only")?;
    let mut m: HashMap<String, i64> = HashMap::new();
    for line in out.lines() {
        let p = line.trim();
        if !p.is_empty() {
            *m.entry(p.to_string()).or_insert(0) += 1;
        }
    }
    Ok(m)
}

/// Indentation complexity per text file: each non-blank line contributes its
/// nesting depth + 1. A cheap, language-agnostic complexity proxy (beats raw line
/// count — it captures nesting, and flat files like markdown score low). At HEAD
/// (`is_head`) it reads the working tree (fast); pinned to a past `anchor` it reads
/// that commit's tree via `git show`, so a file that vanished before HEAD still
/// scores from its then-contents instead of dropping out of the ranking.
pub fn file_complexity(
    repo: &str,
    anchor: &str,
    is_head: bool,
) -> Result<HashMap<String, i64>, String> {
    let mut m = HashMap::new();
    if is_head {
        let files = git(repo, &["ls-files"])?;
        for path in files.lines() {
            if path.is_empty() || is_binary(path) || is_noncode(path) {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(Path::new(repo).join(path)) else {
                continue;
            };
            m.insert(path.to_string(), indent_complexity(&content));
        }
    } else {
        let files = git(repo, &["ls-tree", "-r", "--name-only", anchor])?;
        for path in files.lines() {
            if path.is_empty() || is_binary(path) || is_noncode(path) {
                continue;
            }
            let Ok(content) = git(repo, &["show", &format!("{anchor}:{path}")]) else {
                continue;
            };
            m.insert(path.to_string(), indent_complexity(&content));
        }
    }
    Ok(m)
}

fn indent_complexity(content: &str) -> i64 {
    let mut total = 0;
    for line in content.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let mut indent = 0;
        for ch in line.chars() {
            match ch {
                ' ' => indent += 1,
                '\t' => indent += 4,
                _ => break,
            }
        }
        total += (indent / 4) + 1;
    }
    total
}
