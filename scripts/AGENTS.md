# Scripts Guidance

Read the repository-root `AGENTS.md` first. This file adds the directory map for scripts.

## Directory Map

- `fetch-conformance-inputs.sh`: shared acquisition for hyphenation, Computer Modern fonts, and hash-pinned TRIP/e-TRIP inputs.
- `profile-pdftex-arxiv.sh`: disposable pinned pdfTeX primitive/file-access tracer build and deterministic 100-paper arXiv source profile.
- `select-recent-arxiv.py`: first-submission date filtering and reproducibly random, hash-shuffled candidate selection from the arXiv OAI metadata snapshot ZIP.
- `materialize-recent-arxiv-sample.sh`: parallel source acquisition followed by random-order live-LaTeX filtering and optional durable exclusions for a recent candidate TSV.
- `measure-sharded-manifest.py`: read-only replay of normalized pdfTeX file traces over candidate schema-v2 shard counts.
- `publish-texlive-r2.sh`: verified staged TeX Live snapshot publication to an immutable Cloudflare R2 prefix; browser CORS policy lives beside it in `texlive-r2-cors.json`.
- `test-publish-texlive-r2.sh`: hermetic mock-rclone/curl contract test for resumable, manifest-last R2 publication.
- `run-umber-guarded.py`: canonical process-group watchdog for Umber and tests that execute Umber; enforces wall-time and aggregate-RSS ceilings, TERM-to-KILL escalation, reap, and survivor checks through sandbox-compatible native macOS and Linux process inspection.
- `test-run-umber-guarded.sh`: forced-timeout and RSS-limit self-test proving the shared Umber watchdog kills and reaps descendants.
- `run-stepwise-arxiv-census.sh`: guarded per-paper stepwise-resource census with separate accepted-engine and optional detached-finalizer outcomes.
- `build-texlive-snapshot.sh`: deterministic full TeX Live runtime snapshot staging with package dependency hints and production inventory floors.
- `write-latex-wasm-publish-config.sh`: deterministic schema-3 publisher configuration for the focused LaTeX WASM bundle, pinned to the measured production 8-bit shard policy.
