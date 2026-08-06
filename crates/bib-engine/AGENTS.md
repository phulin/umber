# bib-engine Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the
public facade for the pure-Rust bibliography subsystem. It must remain usable
in native and WASM builds without subprocesses or native-filesystem access.

## File Map

- `Cargo.toml`: crate graph and test-only manifest verification dependencies.
- `src/lib.rs`: detached public job, option, result, failure, attempt, one-shot, and serialization contracts.
- `src/classic.rs`: backend-neutral protocol detection plus bounded classic AUX closure and typed classic resource discovery.
- `src/classic_execution.rs`: classic style compilation, raw database preparation, VM execution, detached artifact routing, and cold/cache parity.
- `src/classic_command.rs`: in-process classic command parsing, status mapping, terminal bytes, and partial-artifact exposure.
- `src/classic_style/`: engine-private classic BST lexer/compiler, compact `READ` arena, shared pool/cache ownership, bounded VM, and focused tests.
- `src/command.rs`: pinned in-process command invocation, output naming, status, terminal, and log-byte adapter.
- `src/command/tests.rs`: exact invocation validation and command-result fixtures.
- `src/session.rs`: resumable VFS resource loop, bounded caches, accepted-input selection, and detached output routing.
- `src/session/convert.rs`: raw BibTeX-to-worker lowering and typed value conversion.
- `src/biber/`: the engine-private typed entry lifecycle, indexed relationship and inheritance pass, sort/label planning, and single frozen-document publication point.
- `src/session/tests.rs`: retry, no-progress, typed-query, and cold/cache parity tests.
- `src/tool.rs`: synthetic-section tool mode and in-process alternate-output routing.
- `tests/it.rs`: the crate's sole Cargo integration-test binary.
- `tests/it/fixtures.rs`: the suite's one host-filesystem seam, and the single
  declared place the `clippy.toml` host-I/O policy is relaxed for reads of
  committed fixtures.
- `tests/it/foundation.rs`: public foundation-boundary tests.
- `tests/it/scaffold.rs`: fixture-manifest, translated-suite census, and
  compatibility-allowance audit.
- `tests/it/upstream/compatibility.rs`: typed Biber case manifest, immutable
  fixture runner, cache-purity replay, and generated-case completeness gate.
- `tests/it/upstream/`: direct, assertion-isolated translations of the pinned
  upstream compatibility suite. Each module contains native Rust assertions
  with identical inputs and expectations; upstream Perl source and Perl
  expression strings are not embedded in the Rust tests. Names, order,
  fixture references, and Unicode remain auditable against the pinned commit.
  Unsupported or currently divergent production behavior is marked on the
  individual test as `#[ignore = "xfail: <specific production gap>"]`; bare
  ignores, weakened expectations, source-presence substitutes, and panic-only
  placeholders are not compatibility tests.

Translated upstream cohorts belong below `tests/it/upstream/` and are modules
of `tests/it.rs`, not additional top-level integration binaries. Public
compatibility tests exercise only `bib-engine` APIs.

## Fixtures and Validation

Pinned upstream bytes live in `tests/corpus/bib/upstream-2.22/`. Ordinary
tests verify and consume those committed bytes hermetically. Regenerate them
only with `scripts/regen-fixtures.sh --area bib`.

Run `cargo test -q --tests -p bib-engine` after changes, followed by the
repository format and clippy gate.
