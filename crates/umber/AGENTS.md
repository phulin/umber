# umber Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the command-line driver and thin public harness for running the engine.

## Crate Role

`umber` wires the engine crates into user-facing commands. The binary currently provides `lex-dump`, `expand-dump`, and `run`; `run` can also write DVI by collecting shipped artifact ids and invoking `tex-out` downstream. The library exposes helpers for preparing primitive state, running in-memory sources, running an already-open input stack, collecting shipout artifact ids, and building DVI bytes from committed artifacts. It owns CLI argument handling, command-specific hooks, job-name/base-directory policy, downstream output-driver composition, and the final effect commit for real runs.

Use this crate when behavior is about driving the engine, presenting CLI output, or providing integration-test harnesses over multiple lower-level crates.

## Boundaries

- Do not put core TeX semantics here; route lexing, expansion, execution, state, typesetting, font, and artifact logic to the owning crates.
- Keep host file access through `World` and command hooks rather than ad hoc reads in lower-level crates.
- Keep CLI output stable enough for integration tests and corpus fixture workflows.
- Avoid widening public helpers unless tests or external callers need the composed engine path.

## File Map

- `AGENTS.md`: crate-local guidance for CLI-driver ownership, boundaries, validation, and this file map.
- `Cargo.toml`: package metadata, feature flags, workspace lint inheritance, and engine/test dependencies.
- `src/expand_dump.rs`: implementation of the `expand-dump` CLI command, including file input hooks and dump primitive setup.
- `src/lib.rs`: public run harness helpers for preparing state, executing input stacks or memory sources, and collecting shipout artifact ids.
- `src/main.rs`: `umber` binary entry point, CLI argument parsing, `lex-dump`/`expand-dump`/`run` dispatch, token formatting, and real-run file hooks.
- `tests/it.rs`: integration-test module root wiring the CLI, replay identity, and effectful replay test suites.
- `tests/it/cli.rs`: integration tests for CLI success, usage errors, corpus dump output, committed pdfTeX diagnostic/DVI fixture parity, and opt-in live hyphenation parity.
- `tests/it/effectful_replay.rs`: property tests for rollback and commit identity across terminal, log, stream, input, read, and shipout effects.
- `tests/it/replay_identity.rs`: property and regression tests that generated primitive programs rollback to identical state.

## Validation

Run `cargo test --tests -p umber` after CLI or composed-runner changes. For behavior that changes emitted diagnostics or fixtures, follow `tests/AGENTS.md` and update fixtures deliberately. Live reference comparisons require `UMBER_LIVE_REF=1` or `scripts/parity.sh`; ordinary tests should consume committed fixtures.
