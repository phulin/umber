# bib-engine Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the
public facade for the pure-Rust bibliography subsystem. It must remain usable
in native and WASM builds without subprocesses or native-filesystem access.

## File Map

- `Cargo.toml`: crate metadata and test-only manifest verification dependencies.
- `src/lib.rs`: public facade root; semantic APIs arrive in later bibliography issues.
- `tests/it.rs`: the crate's sole Cargo integration-test binary.
- `tests/it/support.rs`: strict assertion-level xfail comparisons.
- `tests/it/scaffold.rs`: fixture-manifest and xfail harness self-tests.

Future translated upstream cohorts belong below `tests/it/upstream/` and
should be modules of `tests/it.rs`, not additional top-level integration
binaries. Public compatibility tests exercise only `bib-engine` APIs.

## Fixtures and Validation

Pinned upstream bytes live in `tests/corpus/bib/upstream-2.22/`. Ordinary
tests verify and consume those committed bytes hermetically. Regenerate them
only with `scripts/regen-fixtures.sh --area bib`.

Run `cargo test -q --tests -p bib-engine` after changes, followed by the
repository format and clippy gate.
