# Scripts Guidance

Read the repository-root `AGENTS.md` first. This file adds the directory map for scripts.

## Directory Map

- `fetch-conformance-inputs.sh`: shared acquisition for hyphenation, Computer Modern fonts, and hash-pinned TRIP/e-TRIP inputs.
- `profile-pdftex-arxiv.sh`: disposable pinned pdfTeX primitive/file-access tracer build and deterministic 100-paper arXiv source profile.
- `measure-sharded-manifest.py`: read-only replay of normalized pdfTeX file traces over candidate schema-v2 shard counts.
- `publish-texlive-r2.sh`: verified staged TeX Live snapshot publication to an immutable Cloudflare R2 prefix; browser CORS policy lives beside it in `texlive-r2-cors.json`.
- `test-publish-texlive-r2.sh`: hermetic mock-rclone/curl contract test for resumable, manifest-last R2 publication.
- `run-umber-guarded.py`: canonical process-group watchdog for Umber and tests that execute Umber; enforces wall-time and aggregate-RSS ceilings, TERM-to-KILL escalation, reap, and survivor checks.
- `test-run-umber-guarded.sh`: forced-timeout self-test proving the shared Umber watchdog kills and reaps descendants.
- `run-stepwise-arxiv-census.sh`: guarded per-paper stepwise-resource census with separate accepted-engine and optional detached-finalizer outcomes.
- `build-texlive-snapshot.sh`: deterministic full TeX Live runtime snapshot staging with package dependency hints and production inventory floors.
