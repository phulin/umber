# Scripts Guidance

Read the repository-root `AGENTS.md` first. This file adds the directory map for scripts.

## Directory Map

- `fetch-conformance-inputs.sh`: shared acquisition for hyphenation, Computer Modern fonts, and hash-pinned TRIP/e-TRIP inputs.
- `profile-pdftex-arxiv.sh`: disposable pinned pdfTeX primitive/file-access tracer build and deterministic 100-paper arXiv source profile.
- `select-recent-arxiv.py`: first-submission date filtering and reproducibly random, hash-shuffled candidate selection from the arXiv OAI metadata snapshot ZIP.
- `materialize-recent-arxiv-sample.sh`: parallel source acquisition followed by random-order live-LaTeX filtering and optional durable exclusions for a recent candidate TSV.
- `measure-sharded-manifest.py`: read-only replay of normalized pdfTeX file traces over candidate schema-v2 shard counts.
- `publish-texlive-r2.sh`: verified staged full or HTML-profile publication to distinct immutable Cloudflare R2 prefixes; HTML requires an explicit root pin and publishes `manifest-v4.json`; browser CORS policy lives beside it in `texlive-r2-cors.json`.
- `test-publish-texlive-r2.sh`: hermetic mock-rclone/curl contract test for resumable, manifest-last R2 publication.
- `run-umber-guarded.py`: canonical process-group watchdog for Umber and tests that execute Umber; enforces wall-time and aggregate-RSS ceilings, TERM-to-KILL escalation, reap, and survivor checks through sandbox-compatible native macOS and Linux process inspection.
- `arxiv_corpus.py`: safe exact arXiv archive inventory, identity, verification, and disposable materialization boundary.
- `test-arxiv-corpus.sh`: hermetic archive/view identity contract, including mutation and extra-file rejection.
- `test-run-umber-guarded.sh`: forced-timeout and RSS-limit self-test proving the shared Umber watchdog kills and reaps descendants.
- `check-pdf-external.sh`: opt-in pinned qpdf structural validation plus pinned Poppler raster/text attestation over the representative PDF matrix; `--ci` makes missing tools fatal.
- `run-stepwise-arxiv-census.sh`: stable entry point for the serial guarded arXiv census.
- `stepwise-arxiv-census.py`: single-pass, row-atomic, resumable arXiv census runner and offline evidence verifier.
- `test-stepwise-arxiv-census.sh`: hermetic single-pass, failure-attribution, resume, and verify-only census contract test.
- `archive-stepwise-arxiv-census.py`: validate and archive an exact 100-row guarded census with immutable identities, reference-clean accounting, blocker links, and cluster totals.
- `build-texlive-snapshot.sh`: deterministic full TeX Live runtime snapshot staging with package dependency hints and production inventory floors.
- `build-html-r2.sh`: deterministic two-build staging for the immutable contract-v1 HTML-only R2 profile and curated font catalog.
- `write-latex-wasm-publish-config.sh`: deterministic schema-3 publisher configuration for the focused LaTeX WASM bundle, pinned to the measured production 8-bit shard policy.
- `build-wasm-package.sh`: builds the authored npm runtime with format fixtures
  only; font catalogs and font payload fixtures stay outside the package.
