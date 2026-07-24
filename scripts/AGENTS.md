# Scripts Guidance

Read the repository-root `AGENTS.md` first. This file adds the directory map for scripts.

## Directory Map

- `fetch-conformance-inputs.sh`: shared acquisition for hyphenation, Computer Modern fonts, and hash-pinned TRIP/e-TRIP inputs.
- `profile-pdftex-arxiv.sh`: disposable pinned pdfTeX primitive/file-access
  tracer build for the deterministic recent arXiv sample.
- `measure-wasm-editor-memory.mjs`: deterministic self-contained retained-editor
  workload reporting WebAssembly linear-memory pages before construction, after
  compilation, and after disposal.
- `select-recent-arxiv.py`: first-submission date filtering and reproducibly random, hash-shuffled candidate selection from the arXiv OAI metadata snapshot ZIP.
- `materialize-recent-arxiv-sample.sh`: parallel source acquisition followed by random-order live-LaTeX filtering and optional durable exclusions for a recent candidate TSV.
- `measure-sharded-manifest.py`: read-only replay of normalized pdfTeX file traces over candidate schema-v2 shard counts.
- `publish-texlive-r2.sh`: verified staged full or HTML-profile publication to distinct immutable Cloudflare R2 prefixes; HTML requires an explicit root pin and publishes `manifest-v4.json`; browser CORS policy lives beside it in `texlive-r2-cors.json`.
- `test-publish-texlive-r2.sh`: hermetic mock-rclone/curl contract test for resumable, manifest-last R2 publication.
- `run-umber-guarded.py`: canonical process-group watchdog for Umber and tests that execute Umber; enforces wall-time, aggregate-RSS, and optional progress-file ceilings, TERM-to-KILL escalation, reap, and survivor checks through sandbox-compatible native macOS and Linux process inspection.
- `trip.sh`: guarded TRIP/e-TRIP entry point with documented wall-time, RSS,
  output-progress, fuel, and termination defaults.
- `arxiv_corpus.py`: safe exact arXiv archive inventory, identity, verification,
  and materialization.
- `test-arxiv-corpus.sh`: hermetic archive/view identity contract, including mutation and extra-file rejection.
- `test-run-umber-guarded.sh`: forced-timeout, progress-stall, and RSS-limit self-test proving the shared Umber watchdog kills and reaps descendants.
- `check-pdf-external.sh`: opt-in pinned qpdf structural validation plus pinned Poppler raster/text attestation over the representative PDF matrix; `--ci` makes missing tools fatal.
- `run-stepwise-arxiv-census.sh`: stable entry point for the serial guarded arXiv census.
- `stepwise-arxiv-census.py`: single-pass, row-atomic, resumable arXiv census runner and offline evidence verifier.
- `test-stepwise-arxiv-census.sh`: hermetic single-pass, failure-attribution, resume, and verify-only census contract test.
- `build-texlive-snapshot.sh`: deterministic full TeX Live runtime snapshot staging with package dependency hints and production inventory floors.
- `build-html-r2.sh`: deterministic two-build staging for the immutable contract-v1 HTML-only R2 profile and curated font catalog.
- `write-latex-wasm-publish-config.sh`: deterministic schema-3 publisher configuration for the focused LaTeX WASM bundle, pinned to the measured production 8-bit shard policy.
- `build-wasm-package.sh`: builds the authored npm runtime with format fixtures
  only; font catalogs and font payload fixtures stay outside the package.
- `build-tex82-oracle.sh`: hash-pinned TeX Live source acquisition and
  reproducible clean/instrumentation-ready TeX82 Web2C oracle builds; ordinary
  terminal/log/DVI transparency and the machine-readable semantic event matrix
  are gated together, including the focused expansion/macro/token-list,
  scanner/conditional, and source/input/EOF-recovery program set, and all
  identities are recorded.
- `build-etex26-oracle.sh`: hash-pinned canonical e-TeX 2.6 source acquisition
  and reproducible clean/instrumented Web2C builds; compatibility and extended
  INITEX profiles are separately named, smoke- and schema-v1 base-command
  matrix-gated, audited against the complete canonical extension primitive
  inventory, transparency/effect/repeatability-checked, and recorded.
- `build-pdftex14027-oracle.sh`: hash-pinned canonical pdfTeX 1.40.27 source,
  ordered Web2C/SyncTeX changes, translator, and archive-owned library build;
  publishes separate clean and instrumented eight-bit executables, gates exact
  DVI/PDF smoke plus shared, expansion/scanner, and state/enquiry/effect
  schema-v1 traces, audits the exact canonical primitive inventory, proves
  byte and independently normalized PDF transparency plus determinism, supports
  offline reuse, and records complete build identities.
- `test-oracle-regeneration.sh`: hermetic validation of the pinned three-engine
  regeneration contract, exact canonical profile/fixture selectors, committed
  fixture validation, and schema pins.
