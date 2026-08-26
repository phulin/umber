# Scripts Guidance

The classic BibTeX regeneration branch stages and seals all nine complete case
directories before invoking fixturegen's JSON cohort transaction exactly once.
Generation failures occur before that authority-mutating handoff; shell moves
and sequential rollback are not fixture authority.

`scripts/generate-tex82-property-inventory.py` deterministically regenerates the committed 1,380-module TeX82 inventory after verifying the pinned `tex.web` SHA-256. The catalogue gate supplies the typed default disposition. The generator reads the local source only and never invokes or rewrites the oracle.

Read the repository-root `AGENTS.md` first. This file adds the directory map for scripts.

## Directory Map

- `texlive.py`: import-only authenticated TeX Live 2026 source/runtime library;
  it owns source extraction, linked-worktree source symlinks, metadata-complete
  execution mirrors with selective payloads from hosted or explicit local
  authorities, accepted PDF font-closure positive/negative receipts, and locked
  runtime staging.
- `texlive_release.py`: complete release-runtime acquisition from explicit
  mirror directories, including resumable authenticated TEXMF archive download,
  bounded ISO-range package-database recovery, and atomic primary-tree replacement.
- `provision.py`: the sole provisioning CLI for primary/linked worktrees,
  TeX Live program and release-runtime sources, reference oracles, execution mirrors, and publisher
  snapshots; snapshot publication stages the complete locked
  format-construction closure as the highest-precedence runtime root.
- `test-provision.py`: hermetic program-source and release-runtime acquisition,
  replacement, ISO-range, offline, and ordered TRIP-locator coverage for the
  shared libraries and CLI.
- `measure-wasm-editor-memory.mjs`: deterministic self-contained retained-editor
  workload reporting WebAssembly linear-memory pages before construction, after
  compilation, and after disposal.
- `select-recent-arxiv.py`: first-submission date filtering and reproducibly random, hash-shuffled candidate selection from the arXiv OAI metadata snapshot ZIP.
- `materialize-recent-arxiv-sample.sh`: parallel source acquisition followed by random-order live-LaTeX filtering and optional durable exclusions for a recent candidate TSV.
- `measure-sharded-manifest.py`: read-only replay of normalized pdfTeX file traces over candidate schema-v2 shard counts.
- `publish-texlive-r2.sh`: verified staged full, sparse schema-8 successor, or
  schema-9 HTML-profile publication to Cloudflare R2. Sparse successors reuse
  an authenticated content-addressed base and require a unique root key; HTML
  requires an explicit root pin and publishes `manifest-v9.json`. Browser CORS
  policy lives beside it in `texlive-r2-cors.json`.
- `test-publish-texlive-r2.sh`: hermetic mock-rclone/curl contract test for
  resumable, manifest-last full and sparse-successor R2 publication.
- `test-native-test-assets.py`: hermetic linked-worktree coverage for asset
  identity, source symlinking, isolation, idempotence, and unsafe-input
  rejection through `provision.py`.
- `check-lint-passes.py`: the clippy gate's declared lint passes; verifies each
  pass's feature resolution against Cargo's own records, requires every
  workspace member to be linted and every declared feature's enabled state to
  be either covered or recorded as out of scope, and holds known-dirty
  configurations in an exact, issue-bearing quarantine that fails when it goes
  stale in either direction.
- `test-check-lint-passes.py`: synthetic-input proof that each coverage guard
  in `check-lint-passes.py` fails when it should; the clippy gate runs it
  before trusting them.
- `optional-check-runner.sh`: stateless named-step accounting shared by opt-in
  checks; it runs all selected steps and prints a PASS, PARTIAL, BLOCKED, or
  FAIL verdict.
- `check-tools.sh`, `check-wasm.sh`, `check-hb-shape-fixtures.sh`, and the
  three `check-latex-*.sh` entry points: explicit opt-in checks built on
  `optional-check-runner.sh`. `check-tools.sh profiling-cli` builds and tests
  the feature-gated Umber CLI under the real optimized profiling profile. Run
  one with no arguments for the whole check, or name steps to run exactly
  those. The LaTeX entry points delegate to `run-latex-*.sh` so their
  established implementation options remain available.
- `hooks/`: versioned git hooks installed by `install-hooks.sh` through
  `core.hooksPath`; `pre-commit` runs `check.sh`.
- `run-umber-guarded.py`: canonical process-group watchdog for Umber and tests that execute Umber; enforces wall-time, aggregate-RSS, and optional progress-file ceilings, TERM-to-KILL escalation, reap, and survivor checks through sandbox-compatible native macOS and Linux process inspection.
- `check-and-test.sh`: routine combined gate; prebuilds the complete native test
  suite before clippy can start a second cold Cargo workload, then runs the
  tests under the shared 6 GiB process-group guard concurrently with
  `check.sh`.
- `arxiv_corpus.py`: safe exact arXiv archive inventory, identity, verification,
  and materialization.
- `test-arxiv-corpus.sh`: hermetic archive/view identity contract, including mutation and extra-file rejection.
- `test-run-umber-guarded.sh`: forced-timeout, progress-stall, and RSS-limit self-test proving the shared Umber watchdog kills and reaps descendants.
- `trip-observer-common.sh`: generated-output namespace selection, atomic
  sealed-artifact replacement, and cold-oracle progress heartbeats shared by
  the TeX82/e-TeX observers and their hermetic watchdog/ownership self-test.
- `check-pdf-external.sh`: opt-in pinned qpdf structural validation plus pinned Poppler raster/text attestation over the representative PDF matrix; `--ci` makes missing tools fatal.
- `run-stepwise-arxiv-census.sh`: stable entry point for the serial guarded arXiv census.
- `stepwise-arxiv-census.py`: single-pass, row-atomic, resumable arXiv census runner and offline evidence verifier.
- `test-stepwise-arxiv-census.sh`: hermetic single-pass, failure-attribution, resume, and verify-only census contract test.
- `build-html-r2.sh`: deterministic two-build staging for the immutable contract-v1 HTML-only R2 profile and curated font catalog.
- `write-latex-wasm-publish-config.sh`: deterministic schema-3 publisher configuration for the focused LaTeX WASM bundle, pinned to the measured production 8-bit shard policy.
- `build-wasm-package.sh`: builds the authored npm runtime with format fixtures
  only; font catalogs and font payload fixtures stay outside the package.
- `build-wasm-plain-format.sh`: reproducibly rebuilds and verifies the packaged
  Plain format image from `assets/plain-source.lock`, with every Umber process
  constrained by the shared watchdog.
- `build-initex-format-matrix.sh` and
  `test-build-initex-format-matrix.sh`: serial guarded Plain, LaTeX, and
  pdfLaTeX INITEX reproduction plus hermetic argument-routing coverage.
- `test-build-latex-format.sh`: hermetic required-argument, root-pin,
  all-engine-run passthrough, and forced-offline format-authority coverage.
- `check-latex-representative-resources.sh` and its test: cold, isolated,
  offline source-profile and loaded-format prefetch smokes over the exact
  pdfLaTeX construction/runtime locks, with a machine-readable identity
  receipt.
- `sync-github-issues.sh`: explicit Beads-to-GitHub issue, epic-label, and
  project synchronization helper.
- `build-tex82-oracle.sh`: hash-pinned TeX Live source acquisition and
  reproducible clean/instrumentation-ready TeX82 Web2C oracle builds; ordinary
  terminal/log/DVI transparency and the machine-readable semantic event matrix
  are gated together, including the focused expansion/macro/token-list,
  scanner/conditional, alignment-delivery, and source/input/EOF-recovery
  program set, and all identities are recorded. Its separately built geometry
  profile runs only the focused microfixture and pins schema-v3 hpack, vpack,
  and shipout records.
- `test-tex82-trip-observer.sh`: offline two-phase clean and bounded-profile
  TeX82 TRIP comparison, schema validation, and profile repeatability gate;
  `--geometry-only` provisions pinned reference channels into the selected
  target and prepares deterministic schema-v3 reference geometry without
  running Umber conformance.
- `project-tex82-trip-command.py` and `test-project-tex82-trip-command.py`:
  deterministic bounded-profile projection of TeX82 §483's logical read-stream
  stop name onto the TRIP fixture's legacy physical-terminal contract, with a
  hermetic synthetic regression.
- `test-etex26-trip-observer.sh`: offline two-phase clean, schema-v1 command,
  and schema-v2 geometry e-TeX 2.6 e-TRIP oracle generation and repeatability
  check.
- `build-etex26-oracle.sh`: hash-pinned canonical e-TeX 2.6 source acquisition
  and reproducible clean/instrumented Web2C builds; compatibility and extended
  INITEX profiles are separately named, smoke- and schema-v1 base-command
  matrix-gated, audited against the complete canonical extension primitive
  inventory, transparency/effect/repeatability-checked, and recorded.
- `audit-etex26-extension-primitives.sh`: exact, actionable comparison of the
  canonical `etex.ch` primitive surface against repository ownership and
  extension-matrix coverage.
- `test-etex26-extension-primitive-audit.sh`: hermetic microfixtures for
  accepted exact coverage and actionable missing, extra, and unowned
  command-core boundaries.
- `build-pdftex14029-oracle.sh`: hash-pinned canonical pdfTeX 1.40.29 source,
  ordered Web2C/SyncTeX changes, translator, and archive-owned library build;
  publishes separate clean and instrumented eight-bit executables, gates exact
  DVI/PDF smoke plus shared, expansion/scanner, and state/enquiry/effect
  schema-v1 traces, including an exact extended-versus-compatibility e-TeX
  boundary matrix with focused recovery and executor/list-state jobs, audits
  the exact canonical primitive inventory, proves
  byte and independently normalized PDF transparency plus determinism, supports
  offline reuse, and records complete build identities.
- `test-oracle-regeneration.sh`: hermetic validation of the pinned three-engine
  regeneration contract, exact canonical profile/fixture selectors, committed
  fixture and bidirectional TeX82 matrix audit, full-document staged
  publication, and schema pins.
