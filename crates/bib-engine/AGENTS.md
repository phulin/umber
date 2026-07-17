# bib-engine Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the
public facade for the pure-Rust bibliography subsystem. It must remain usable
in native and WASM builds without subprocesses or native-filesystem access.

## File Map

- `Cargo.toml`: crate graph and test-only manifest verification dependencies.
- `src/lib.rs`: detached public job, option, result, failure, attempt, one-shot, and serialization contracts.
- `src/session.rs`: resumable VFS resource loop, bounded caches, stage composition, and detached output routing.
- `src/session/convert.rs`: raw BibTeX-to-model conversion, typed values, and label-source preparation.
- `src/session/tests.rs`: retry, no-progress, typed-query, and cold/cache parity tests.
- `src/tool.rs`: synthetic-section tool mode and in-process alternate-output routing.
- `tests/it.rs`: the crate's sole Cargo integration-test binary.
- `tests/it/foundation.rs`: public foundation-boundary tests.
- `tests/it/support.rs`: strict assertion-level xfail comparisons.
- `tests/it/scaffold.rs`: fixture-manifest and xfail harness self-tests.
- `tests/it/upstream/`: direct, assertion-isolated translations of the pinned
  upstream compatibility suite. Each module retains the complete upstream
  source beside its Rust xfails so names, order, expressions, fixture
  references, and Unicode stay auditable.

Translated upstream cohorts belong below `tests/it/upstream/` and are modules
of `tests/it.rs`, not additional top-level integration binaries. Public
compatibility tests exercise only `bib-engine` APIs.

## Fixtures and Validation

Pinned upstream bytes live in `tests/corpus/bib/upstream-2.22/`. Ordinary
tests verify and consume those committed bytes hermetically. Regenerate them
only with `scripts/regen-fixtures.sh --area bib`.

Run `cargo test -q --tests -p bib-engine` after changes, followed by the
repository format and clippy gate.
