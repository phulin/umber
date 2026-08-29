# Repository Guidance

This project is a faster, more modern, more portable reimplementation of TeX, LaTeX, and pdfTeX in Rust.

The repository uses progressive disclosure: read this file first, then the nearest nested `AGENTS.md` before editing within a subdirectory; keep every `AGENTS.md` up to date whenever source files or subdirectories are added, removed, or repurposed.

The project also uses bd (beads) for issue tracking; see below for full instructions.

## General Instructions

- Commit as you go in logical chunks. Write good commit messages (a one-line summary and then details below). Use rough Conventional Commits style. You have
  to escalate privileges to commit.
- Make sure you are writing clean code; don't hesitate to do refactor commits if you find that a certain area of the code has gotten complex or difficult to understand.
- Don't worry about keeping changes "low-risk" or implementing only "narrow slices", as making clean code will sometimes require big, ambitious, cross-cutting changes, and reimplementing something from scratch means we will need to write complex new features.
- In general, try to keep source files short (goal is under roughly 600 lines, but it's okay if a file gets somewhat larger; test files can be as long as needed, they should only be split logically).
- When writing code, prefer principled solutions, clean architecture, and fast, optimized implementation. Avoid hacks.
- Adding or renaming a Cargo feature: read
  [Cargo Feature Axes](docs/cargo_feature_axes.md) first. It names the four
  axes a feature may belong to, the one crate that owns each declaration, and
  the gate declaration a new feature must be added to before it can pass.
- For complex features, build design/technical documentation in advance and place in docs/ for your own planning and for reference later, but don't commit temporary task plans or notes.
- Prefer `#[cfg(test)] mod tests;` with separate `src/.../tests.rs` files for nontrivial crate-internal tests. If writing tests, read `docs/testing_policy.md`.
- Make sure you can run the test suite very quickly so we don't gate our progress on test su ite speed. Run `cargo test` with `--tests` so you don't run the doctests.
- Limit `rg` output aggressively - you can easily fill up your context with it.
- Codex: For `wait` and `write_stdin`, schedule timeout of at least 180s, and for `wait_agent`, 600s.
- To the extent possible, combine adjacent commands into one (e.g. use && to sequence a commit IF tests pass).
- Working a canonical-command (`umber2-johp`) semantic or DVI divergence: read
  [Canonical Divergence Working Contract](docs/canonical_divergence_workflow.md)
  first. It is the working contract (oracle hierarchy, diagnosis order, fix
  discipline, gates) and supersedes ad hoc dispatch-prompt narration.

## Directory Map

- `.cargo/`: target-specific Cargo configuration; browser randomness selection and the 4 MiB engine stack must remain scoped to `wasm32-unknown-unknown`.
- `.agents/`: project-local agent skills and coordination workflow guidance.
- `crates/`: Rust workspace crates.
- `crates/tex-arith`: shared TeX scaled-point and TFM arithmetic.
- `crates/tex-command`: canonical raw command delivery, expansion, scanning,
  conditions, alignments, and profile dispatch (currently the architectural
  boundary skeleton).
- `crates/tex-content`: shared versioned, domain-separated content identity.
- `crates/tex-oracle`: versioned canonical semantic-event schema, fixture
  identity, normalization, and detached instrumentation transport.
- `crates/tex-observe`: detached command-observation translation into portable
  normalized semantic and geometry evidence.
- `crates/tex-state`: engine state layer substrate.
- `crates/tex-state/profiling-allocator`: profiling-only global-allocation
  attribution shim for named hot-core scopes; engine crates remain unsafe-free.
- `crates/tex-fonts`: immutable validated font contexts, metrics, and OpenType shaping.
- `crates/tex-exec`: stomach execution, mode nest, main-control dispatch, assignments, and h/v-mode material construction.
- `crates/tex-incr`: named-boundary editor sessions, revision mapping, convergence, pruning, and suffix reuse.
- `crates/tex-typeset`: pure packing, line-breaking, and list transformation kernels.
- `crates/tex-out`: committed page artifact model, hashing, and binary serialization.
- `crates/umber-vfs`: host-neutral canonical virtual paths and shared virtual filesystem substrate.
- `crates/umber`: CLI driver.
- `crates/umber-wasm`: WebAssembly binding and authored JavaScript browser package.
- `crates/test-support`: shared fixture and parity-test helpers.
- `crates/corpus-manifest`: dependency-free parser for the external corpus manifest used by host-side parity tooling.
- `crates/umber-distribution`: dependency-free immutable distribution manifest parsing, request-key encoding, and acquisition selection.
- `crates/umber-fetch`: native content-addressed distribution cache and bounded blocking HTTPS acquisition.
- `crates/umber-hash`: portable, versioned deterministic aHash64 identities shared by persisted distribution and output contracts.
- `crates/umber-interrupt`: repository-owned safe Ctrl-C registration and platform signal dispatch.
- `crates/bib-model`: typed immutable bibliography values, builders, options, diagnostics, and frozen documents.
- `crates/bib-unicode`: pinned immutable Unicode compatibility resource boundary.
- `crates/bib-input`: control, configuration, and datasource input-stage boundary.
- `crates/bib-output`: detached deterministic serializer boundary.
- `crates/bib-engine`: public bibliography facade, engine-private Biber and classic BibTeX runtimes, and pinned upstream compatibility suite.
- `tools/`: Rust tooling crates.
- `benchmarks/`: opt-in standalone benchmark crates kept outside the root workspace.
- `benchmarks/edit-restart`: paired Plain TeX edit workloads for incremental restart measurement.
- `benchmarks/format-restore`: focused schema-11 loaded-format decode work and allocation benchmark.
- `benchmarks/tex-command`: command-core allocation and packed-cutover gates.
- `benchmarks/tex-exec`: focused shipout-lowering diagnostic.
- `benchmarks/tex-incr`: accepted-edit pure-memo diagnostic.
- `benchmarks/tex-state`: snapshot/state performance gates, focused PDF and
  frozen-hyphenation checkpoint gates, checked page-span allocation/copy
  gates, transactional candidate-family gates, and state diagnostics.
- `benchmarks/tex-typeset`: pure layout, allocation, and compact-width gates.
- `tests/`: committed fixtures and parity test definitions.
- `tests/corpus/pdf/`: pinned minimal pdfTeX references, deterministic Umber PDFs, normalized structure, and rendered-page parity fixtures.
- `docs/`: architecture, phase, and design documents.
- `docs/expansion_memory_lifetimes.md`: implementation map and retention audit
  for expansion generations, durable values, scratch, scanners, suspension,
  input, effects, and format ownership.
- `docs/page_region_span_validation.md`: ownership-boundary validation map for
  page-list coordinates, checked traversal spans, rollback, succession,
  shipout, and retirement.
- `docs/node_region_ownership.md`: authoritative exclusive node-region design
  for exact paragraph checkpoints, page succession, TeX move/copy semantics,
  and whole-region reclamation.
- `scripts/`: local development scripts and versioned git hook templates.
- `third_party/`: ignored reference downloads and external source archives.

## Development

- Implementation agents should run the relevant tests explicitly, then use
  `scripts/check.sh` for the format and clippy gate without rerunning tests.
  It runs every gate even after one fails and ends with a verdict line naming
  the failures; that line, not the absence of scrollback, is the result to
  report.
- `scripts/check.sh` is the only thing that may be reported as a gate result.
  To run one gate alone, name it: `scripts/check.sh clippy` runs byte-identical
  commands to the full run. Never hand-write a `cargo clippy` invocation and
  call the result "clippy clean": a bare `cargo clippy` exits 0 on warn-level
  lints such as the `clippy.toml` host-I/O policy, and any one invocation lints
  a single feature resolution, while the gate lints every resolution the tree
  is built in, so a hand-written run can look green while the gate is red.
- When running tests, make sure to use `cargo test -q` so you don't fill up
  your context window.
- **`cargo test --tests` is the whole routine suite.** `default-members` lists
  every host-testable workspace member, so no wrapper script selects them and
  none is needed. It once listed only 21 of 34, which left the nine `bib-*`
  crates, `umber-interrupt`, `refexec`, and `profile-analyzer` executed by no
  routine command at all (`umber2-johp.211`);
  `default_members_cover_every_host_testable_crate` in `test-support` now fails
  if that list drifts from the workspace again, so the coverage is enforced by
  the suite rather than by remembering which command to type. `umber-wasm` is
  the sole omission and declares its tier in that test.
- **Provision conformance assets when creating or allocating a worktree.** The
  byte-exact DVI oracles, TRIP inputs, and shared font/hyphenation inputs are
  gitignored for licensing reasons. Before running tests in a linked worktree,
  run `python3 scripts/provision.py worktree <worktree>`. It copies only the
  `tests/native-test-assets.lock` allowlist from the primary checkout, verifies
  every SHA-256 on both sides, and leaves the copies ignored. Rust tests never
  provision their own inputs. Do not manually link or broadly copy
  `third_party/`.

  If the **primary checkout** lacks an asset there is nothing to copy from, and
  the gates fail with the missing paths named. Materialize them once, in the
  primary checkout, not in a worktree:

  ```bash
  python3 scripts/provision.py worktree .
  ```

  That command downloads the pinned TeX Live 2026 source and runtime inputs,
  builds the instrumented pdfTeX 1.40.29 oracle, and generates the locked
  fixtures. Linked worktrees symlink the primary source archive/tree and copy
  only the locked native assets. An environment that genuinely
  cannot host the oracles opts out explicitly with
  `UMBER_CONFORMANCE_ORACLES=optional`, which downgrades the byte-exact gates
  to a loud notice rather than letting an absent oracle read as a pass.
- Direct `cargo build` output to a log file; it has verbose output.
- Use `scripts/check-and-test.sh` when a single command should run the default
  native correctness suite concurrently with the format and clippy gate.
  Clippy uses its own `target/clippy` directory so it does not lock the test
  build.
- Cargo targets are checkout-local by default; persistent linked worktrees keep
  their own compiled artifacts between issues.
- Use `cargo run-dev -p umber -- <args>` for local CLI runs that should share
  optimized artifacts with the test build.
- Snapshot-sensitive corpus and format work must pass the explicit regenerated
  2026-03-01 distribution path to Umber (normally
  `--distribution target/texlive-snapshot`, resolved from the owning checkout)
  plus its authenticated `--distribution-ahash64` pin and must not rely on the
  default hosted manifest. The native cache is shared
  and content-addressed rather than snapshot-partitioned: stop concurrent Umber
  runs before purging its `objects`/`manifests` namespaces, then warm only from
  the explicit pinned distribution and verify offline reuse.

### Writing Markdown

`scripts/check.sh` runs `dprint`, which reformats every `.md` file in the
repository. Its markdown plugin rewrites content, not just layout, so markdown
that documents TeX must be written in the shape dprint already agrees with.
`dprint check` passing is the proof that no file holds a construct dprint would
rewrite; never silence it with an ignore directive or a plugin exclusion.

- Never let an inline code span begin or end with a space: dprint deletes it,
  and no backtick fencing escapes this. Write a literal space inside a code
  span as `␣` (U+2423 OPEN BOX), so TeX's control space is `\␣` and e-TeX's
  pseudo-file trace string is `(␣`. A bare `\` means the escape character.
- Keep backticks balanced per paragraph. TeX opens its quoted error text with a
  backtick, so a message such as ``You can't use `\eqno' in ... mode`` has to
  sit inside a double-backtick span; a backtick left open makes dprint read the
  prose as the code span and the code as prose, then eat the spaces between
  words.
- Do not indent fenced-code content past its own fence: dprint strips the
  block's common leading indentation.
- Keep a whole `[text](target)` link on one source line; dprint joins link text
  that wraps, however long the resulting line is.
- Use `_emphasis_`, not `*emphasis*`.

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
