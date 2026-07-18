# Repository Guidance

This project is a faster, more modern, more portable reimplementation of TeX, LaTeX, and pdfTeX in Rust.

The repository uses progressive disclosure: read this file first, then the nearest nested `AGENTS.md` before editing within a subdirectory; keep every `AGENTS.md` up to date whenever source files or subdirectories are added, removed, or repurposed.

The project also uses bd (beads) for issue tracking; see below for full instructions.

## General Instructions

- Commit as you go in logical chunks. Write good commit messages (a one-line summary and then details below). You have to escalate privileges to commit.
- Make sure you are writing clean code; don't hesitate to do refactor commits if you find that a certain area of the code has gotten complex or difficult to understand.
- Don't worry about keeping changes "low-risk" or implementing only "narrow slices", as making clean code will sometimes require big, ambitious, cross-cutting changes, and reimplementing something from scratch means we will need to write complex new features.
- In general, try to keep source files short (goal is under roughly 600 lines, but it's okay if a file gets somewhat larger; test files can be as long as needed, they should only be split logically).
- When writing code, prefer principled solutions, clean architecture, and fast, optimized implementation. Avoid hacks.
- For complex features, build design/technical documentation in advance and place in docs/ for your own planning and for reference later, but don't commit temporary task plans or notes.
- Prefer `#[cfg(test)] mod tests;` with separate `src/.../tests.rs` files for nontrivial crate-internal tests. If writing tests, read `docs/testing_policy.md`.
- Make sure you can run the test suite very quickly so we don't gate our progress on test su ite speed. Run `cargo test` with `--tests` so you don't run the doctests.
- Limit `rg` output aggressively - you can easily fill up your context with it.
- Codex: for `wait`, schedule timeout of at least 180s, and for `wait_agent`, 600s.

## Directory Map

- `.cargo/`: target-specific Cargo configuration; browser randomness selection and the 4 MiB engine stack must remain scoped to `wasm32-unknown-unknown`.
- `.agents/`: project-local agent skills and coordination workflow guidance.
- `crates/`: Rust workspace crates.
- `crates/tex-arith`: shared TeX scaled-point and TFM arithmetic.
- `crates/tex-content`: shared versioned, domain-separated content identity.
- `crates/tex-state`: engine state layer substrate.
- `crates/tex-fonts`: immutable font metric parsing and TFM data.
- `crates/tex-lex`: line normalization, tokenization, input stack, and token-list replay.
- `crates/tex-expand`: gullet expansion, expandable primitives, conditionals, and value scanners.
- `crates/tex-exec`: stomach execution, mode nest, main-control dispatch, assignments, and h/v-mode material construction.
- `crates/tex-incr`: named-boundary editor sessions, revision mapping, convergence, pruning, and suffix reuse.
- `crates/tex-typeset`: pure packing, line-breaking, and list transformation kernels.
- `crates/tex-shape`: pure Unicode/OpenType single-run shaping and positioned glyph output.
- `crates/tex-out`: committed page artifact model, hashing, and binary serialization.
- `crates/umber-vfs`: host-neutral canonical virtual paths and shared virtual filesystem substrate.
- `crates/umber`: CLI driver.
- `crates/umber-wasm`: WebAssembly binding and authored JavaScript browser package.
- `crates/test-support`: shared fixture and parity-test helpers.
- `crates/corpus-manifest`: dependency-free parser for the external corpus manifest used by host-side parity tooling.
- `crates/umber-distribution`: dependency-free immutable distribution manifest parsing, request-key encoding, and acquisition selection.
- `crates/umber-fetch`: native content-addressed distribution cache and bounded blocking HTTPS acquisition.
- `crates/bib-model`: typed immutable bibliography values, builders, options, diagnostics, and frozen documents.
- `crates/bib-unicode`: pinned immutable Unicode compatibility resource boundary.
- `crates/bib-input`: control, configuration, and datasource input-stage boundary.
- `crates/bib-graph`: relationship, inheritance, set, and validation-stage boundary.
- `crates/bib-sort`: data-list, filtering, and stable sorting-stage boundary.
- `crates/bib-label`: label, hash, and uniqueness-stage boundary.
- `crates/bib-output`: detached deterministic serializer boundary.
- `crates/bib-engine`: public bibliography facade and pinned upstream compatibility suite.
- `crates/bib-bst`: bounded classic BibTeX style lexer, parser, compiler, and immutable programs.
- `tools/`: Rust tooling crates.
- `benchmarks/`: opt-in standalone benchmark crates kept outside the root workspace.
- `tests/`: committed fixtures and parity test definitions.
- `tests/corpus/pdf/`: pinned minimal pdfTeX references, deterministic Umber PDFs, normalized structure, and rendered-page parity fixtures.
- `docs/`: architecture, phase, and design documents.
- `scripts/`: local development scripts and versioned git hook templates.
- `scripts/profile-pdftex-arxiv.sh`: disposable pinned pdfTeX primitive/file-access tracer build and deterministic 100-paper arXiv source profile.
- `scripts/measure-sharded-manifest.py`: read-only replay of normalized pdfTeX file traces over candidate schema-v2 shard counts.
- `scripts/publish-texlive-r2.sh`: verified staged TeX Live snapshot publication to an immutable Cloudflare R2 prefix; browser CORS policy lives beside it in `scripts/texlive-r2-cors.json`.
- `scripts/test-publish-texlive-r2.sh`: hermetic mock-rclone/curl contract test for resumable, manifest-last R2 publication.
- `scripts/build-texlive-snapshot.sh`: deterministic full TeX Live runtime snapshot staging with package dependency hints and production inventory floors.
- `third_party/`: ignored reference downloads and external source archives.

## Development

- Implementation agents should run the relevant tests explicitly, then use
  `scripts/check.sh` for the format and clippy gate without rerunning tests.
- When running tests, make sure to use `cargo test -q` so you don't fill up
  your context window.
- Direct `cargo build` output to a log file; it has verbose output.
- Use `scripts/check-and-test.sh` when a single command should run the default
  native correctness suite concurrently with the format and clippy gate.
  Clippy uses its own `target/clippy` directory so it does not lock the test
  build.
- Use `cargo run-dev -p umber -- <args>` for local CLI runs that should share
  optimized artifacts with the test build.
- Run `scripts/check-snapshot-budgets.sh` in the explicit performance tier;
  snapshot allocation and latency gates do not run under ordinary cargo tests.
- Run `scripts/check-tools.sh` for the explicit host-side regeneration,
  profiling, and triage-tool test/clippy gate; these targets are excluded from
  routine native correctness builds.

## Beads Issue Tracker

Use Beads (`bd`) for durable task tracking in repositories that include it. Use the `beads` skill for more detailed Beads workflow guidance, then use the `bd` CLI for issue operations.

### Quick Reference

```bash
bd ready                # Find available work
bd show <id>            # View issue details
bd update <id> --claim  # Claim work
bd close <id>           # Complete work
```

### Rules

- Use `bd` for all task tracking; do not create markdown TODO lists.
- Keep persistent project memory in Beads via `bd remember`; do not create ad hoc memory files.

**Architecture in one line:** issues live in a local Dolt DB; sync uses `refs/dolt/data` on your git remote; `.beads/issues.jsonl` is a passive export. See https://github.com/gastownhall/beads/blob/main/docs/SYNC_CONCEPTS.md for details and anti-patterns.
