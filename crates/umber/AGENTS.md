# umber Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the command-line driver and thin public harness for running the engine.

## Crate Role

`umber` wires the engine crates into user-facing commands. The binary currently provides `lex-dump`, `expand-dump`, and `run`; `run` can also write DVI by collecting shipped artifact ids and invoking `tex-out` downstream. The library exposes the shared engine-session orchestration boundary, file search hooks, typed finalization phases, in-memory helpers, and DVI construction from committed artifacts. It owns CLI argument handling, job-name/base-directory policy, downstream output-driver composition, and the final effect commit for real runs.

Use this crate when behavior is about driving the engine, presenting CLI output, or providing integration-test harnesses over multiple lower-level crates.

## Boundaries

- Do not put core TeX semantics here; route lexing, expansion, execution, state, typesetting, font, and artifact logic to the owning crates.
- Keep host file access through `World` and command hooks rather than ad hoc reads in lower-level crates.
- Keep CLI output stable enough for integration tests and corpus fixture workflows.
- Avoid widening public helpers unless tests or external callers need the composed engine path.

## File Map

- `AGENTS.md`: crate-local guidance for CLI-driver ownership, boundaries, validation, and this file map.
- `Cargo.toml`: package metadata, feature flags, workspace lint inheritance, and engine/test dependencies.
- `src/expand_dump.rs`: implementation of the `expand-dump` CLI command through the shared engine session and dump primitive setup.
- `src/input_search.rs`: deterministic driver-owned TeX input and TFM font path resolution through World-backed reads.
- `src/input_search/tests.rs`: focused TeX input/font area ordering, extension, and input-record coverage.
- `src/lib.rs`: shared engine session, file hooks, typed effect-before-driver finalization, run helpers, and one-artifact-at-a-time DVI construction.
- `src/memory_output.rs`: exact committed terminal/log/DVI/aux collection for successful memory-backed runs with aggregate output limits.
- `src/memory_output/tests.rs`: final-commit idempotence, output accounting, and memory-boundary tests.
- `src/virtual_compile.rs`: host-neutral restart-on-fetch session, typed request/result API, deterministic cache, and resource accounting.
- `src/virtual_compile/path.rs`: POSIX-like `/job` and `/texlive` path validation plus logical TeX/TFM request normalization.
- `src/virtual_compile/hooks.rs`: World-backed execution hooks with ordered typed missing-file side state.
- `src/virtual_compile/tests.rs`: native retry, path, precedence, limits, format, effect-isolation, font batching, and DVI coverage.
- `src/main.rs`: `umber` binary entry point, CLI argument parsing, `lex-dump`/`expand-dump`/`run` dispatch, token formatting, and real-run file hooks.
- `tests/it.rs`: integration-test module root wiring CLI, replay identity, effectful replay, and end-to-end conformance suites.
- `tests/it/cli.rs`: integration tests for CLI success, usage errors, corpus dump output, and committed diagnostic/DVI fixture parity.
- `tests/it/e2e_conformance.rs`: individually selectable Story, Gentle, TRIP, and e-TRIP tests that execute Umber in process against gitignored, locally generated `tests/corpus/e2e` DVI oracles through `parity-harness`; TRIP and e-TRIP share one two-phase format helper, and each case runs conditionally when its external inputs and oracle exist.
- `tests/it/effectful_replay.rs`: property tests for rollback and commit identity across terminal, log, stream, input, read, and shipout effects.
- `tests/it/replay_identity.rs`: property and regression tests that generated primitive programs rollback to identical state.

## Validation

Run `cargo test --tests -p umber` after CLI or composed-runner changes. For behavior that changes emitted diagnostics or fixtures, follow `tests/AGENTS.md` and regenerate deliberately with `scripts/regen-fixtures.sh`. Ordinary corpus tests consume committed fixtures; external end-to-end conformance tests conditionally consume locally generated oracles.
