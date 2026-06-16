# Terminal Velocity (`tv`)

A `git status` for build-flow health: read commit history, print a one-screen
status board — a few self-calibrated leading indicators, each with a status glyph
you triage at a glance. Zero-dependency Rust, deterministic, offline. The product
rationale and the design non-negotiables live in `README.md`. This file is how to
*work in the code* without breaking them.

## Invariants — don't regress these

- **Aggregate by subsystem, never by person.** The load-bearing safety rule.
  `thrash` / `hotspots` / `cadence` group by directory or time, never by author.
  The one sanctioned exception is `--me` — self-instrumentation from the running
  user's own `git config`. There is deliberately no `--author=<someone-else>`;
  that is the surveillance tool the field forbids. Don't add it, and don't add any
  per-teammate breakdown.
- **Zero runtime dependencies, std-only.** Nothing in `[dependencies]`. Color is
  hand-rolled ANSI (`style.rs`); arg parsing is hand-rolled (`main.rs`). It must
  build offline with nothing to fetch.
- **Deterministic & offline.** Same repo + same anchor → same output. No network,
  no LLM at runtime. Each metric's status is *rule-composed* from its own data
  (`metrics.rs`), never generated. *Presentation* adapts to the terminal — color
  and board width (`Palette`, detected via `stty`) track the tty — but piped /
  non-tty output uses fixed defaults (80 cols, no color), so scripted runs stay
  byte-stable. Data is deterministic; only the frame around it flexes.
- **Status, never a verdict.** The board shows every metric with equal billing,
  each tagged with a left-gutter status glyph (`Tone::glyph` — `·` calm / `✓` good
  / `▲` watch / `■` alarm; symbol-distinct so it survives `NO_COLOR` and colorblind
  eyes — color only reinforces). The reader's eye triages; an all-`·` gutter reads
  as "clean", git-status-style. Deliberately **no composed prose verdict**: any
  one-line summary is lossy and editorializes *what matters* (selective bias), the
  same sin as ranking people. Coverage honesty (`< BASELINE_WEEKS` of history →
  `provisional` chip) is the one global caveat, and it rides the header, not a
  metric. (This retired the old `verdict.rs` composer.)
- **The classifier is a future seam, never a runtime requirement.** A learned/DLM
  intent classifier may *sharpen* labels later via an external call, but `tv` must
  always run fully without it. `intent.rs` is the heuristic; keep the swap-point
  clean — don't let anything depend on the call.

## How code is written here

- **No arbitrary numeric defaults.** The spine of the tool. Don't hard-code
  top-N, day windows, or thresholds. Self-calibrate from the data: survival
  `S(age)` is the repo's own yardstick (no magic N-day window); drill-in cuts use
  the Pareto / cumulative-share principle (`metrics::VITAL_FEW`, `pareto_count`),
  not top-N. A genuinely needed constant gets one documented home and a rationale.
- **Fail loud.** Required git output is required — surface a named error
  (`{repo}: {e}`), don't paper over with a silent default. A typo'd `--at` anchor
  must never quietly resolve to "now" (`looks_like_date` guards this).
- **Git is the only data source, via plumbing.** Everything comes from
  `git log --numstat`, `ls-tree`, `blame`, `rev-list`, … shelled out from
  `git.rs`. No other inputs.
- **Cache discipline.** The blame-at-death pass is cached as text under
  `<gitdir>/tv-cache`, keyed by the anchor sha (HEAD → `tv-cache`; a pinned `--at`
  → `tv-cache-<sha12>`, so archaeology never clobbers the everyday cache). Any
  change to what's stored bumps the `TVCACHE<n>` version tag so stale caches
  recompute once. Caching changes speed, never results.
- **Layers, one direction.** `main` (CLI, `--at` resolution, dispatch) → `metrics`
  (cockpit / thrash tree / hotspots) → `git` (plumbing) + `survival` / `spark` /
  `intent` (math) + `model` (types) → `render` (terminal + HTML). Keep the arrows
  pointing down. Survival is fit **per repo** — repo frailty dominates line
  survival, so a pooled curve misweights every repo.
- **Communicate the anchor.** Every time-framed surface must read correctly under
  `--at` — none may imply "now" when it isn't (`render::lens_window`,
  `Cockpit.as_of`).

## The gate

`mise run check` = `fmt-check` + `clippy --all-targets -D warnings` + `test`. It
must be green before any milestone is "done." `mise run fmt` fixes formatting.
Toolchain is pinned in `rust-toolchain.toml` (stable).

## Releasing

Dev loop: edit → `mise run check` → commit. CI runs the same gate.

Cut a release:
1. Bump `version` in `Cargo.toml`.
2. `mise run check` — also re-syncs `Cargo.lock` to the new version. **Don't skip:**
   CI builds with `--locked`, so a stale lock fails every platform.
3. Commit (incl. `Cargo.lock`) and push.
4. Tag it, matching the version exactly: `git tag v0.1.1 && git push origin v0.1.1`.

`.github/workflows/release.yml` then builds 5 native targets (Intel macOS is
cross-compiled on Apple Silicon — no Intel runners) → draft release → publishes
when all pass → auto-bumps the Homebrew formula in `onebit0fme/homebrew-tap`
(needs the `HOMEBREW_TAP_TOKEN` secret).

## Working with the user

- **The user manages git.** Never run `git checkout -b`, `git commit`, `git push`,
  or `git branch`. Edit in place. Ask before any destructive op.
- **Milestone-review workflow.** At a green increment, summarize what changed,
  suggest a commit message, and **pause** — the user commits in a separate
  session. Suggested commit messages omit `Co-Authored-By` trailers.
- **Verify against git before "fixing" a metric.** When a number looks wrong,
  check it against the actual history first — the tool has been right and memory
  wrong.
