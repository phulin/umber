# umber Guidance

Read the repository-level `AGENTS.md` before editing here. This crate is the command-line driver and thin public harness for running the engine.

## Crate Role

`umber` wires the engine crates into user-facing commands. The binary provides `lex-dump`, `expand-dump`, `bib`, and `run`; `bib` stages native files for the in-process bibliography adapter, while `run` composes TeX82, e-TeX, pdfTeX, LaTeX-DVI, and pdfLaTeX engine layers and can publish DVI or PDF from committed artifacts. The library exposes the shared engine-session orchestration boundary, localized file resolvers, typed finalization phases, in-memory helpers, and downstream artifact construction. It owns CLI argument handling, job-name/base-directory policy, engine capability composition, downstream output-driver composition, and the final effect commit for real runs.

Use this crate when behavior is about driving the engine, presenting CLI output, or providing integration-test harnesses over multiple lower-level crates.

## Boundaries

- Do not put core TeX semantics here; route lexing, expansion, execution, state, typesetting, font, and artifact logic to the owning crates.
- Keep host file access through `World` and command resolvers rather than ad hoc reads in lower-level crates.
- Keep CLI output stable enough for integration tests and corpus fixture workflows.
- Avoid widening public helpers unless tests or external callers need the composed engine path.

## File Map

- `AGENTS.md`: crate-local guidance for CLI-driver ownership, boundaries, validation, and this file map.
- `Cargo.toml`: package metadata, feature flags, workspace lint inheritance, and engine/test dependencies.
- `src/expand_dump.rs`: implementation of the `expand-dump` CLI command through the shared engine session and dump primitive setup.
- `src/format_cache_cli.rs`: pinned LaTeX/pdfLaTeX generated-format cache identity, validated restore, and atomic publication CLI adapter.
- `src/bib.rs`: native host-file staging, resource retry, and detached artifact publication for the in-process `bib` command.
- `src/classic_bib.rs`: native host-file staging and artifact publication for the in-process classic `bibtex` command.
- `src/input_search.rs`: deterministic driver-owned TeX input and TFM font path resolution through World-backed reads.
- `src/input_search/tests.rs`: focused TeX input/font area ordering, extension, and input-record coverage.
- `src/latex_project.rs`: host-neutral transactional LaTeX/bibliography multipass orchestration, convergence, and atomic project acceptance.
- `src/latex_project/support.rs`: project candidate VFS assembly, generated-file identity, and shared resource conversion helpers.
- `src/latex_project/tests.rs`: project convergence, bibliography publication, and rollback coverage.
- `src/html_output.rs`: exact native typed font-resource resolver with TFM identity, WOFF2 digest, complete legacy mapping, and embedding-license validation.
- `src/lib.rs`: shared engine session, file resolvers, typed effect-before-driver finalization, run helpers, and one-artifact-at-a-time DVI construction.
- `src/memory_output.rs`: exact committed terminal/log/DVI/aux collection for successful memory-backed runs, aggregate output limits, and auxiliary publication into VFS stage transactions.
- `src/memory_output/tests.rs`: final-commit idempotence, output accounting, and memory-boundary tests.
- `src/pdf_import.rs`: lightweight PDF syntax inspection and lossless selected-page resource import through `hayro-syntax`.
- `src/pdf_import/tests.rs`: synthetic and conditional pinned-corpus PDF import regressions.
- `src/pdftex.rs`: pinned pdfTeX 1.40.27 primitive inventory and explicit placeholder registration for pdfTeX mode.
- `src/pdf_output.rs`: deterministic committed-artifact lowering into the checkpointed PDF object graph.
- `src/pdf_font_resources_tests.rs`: post-acceptance real-font fallback and virtual-root exclusion tests.
- `src/pdf_vf.rs`: bounded recursive virtual-font packet lowering into detached PDF-positioned operations and real-font resources.
- `src/pdf_vf/tests.rs`: synthetic packet execution, recursion, resource-selection, and lowering-limit tests.
- `src/virtual_compile.rs`: host-neutral persistent compile session, versioned mapped-TFM layout policy, revision-checked root patches, shared-VFS file/OpenType resource retries, atomic response registration, retained immutable resources, and composed resource accounting.
- `src/virtual_compile/path.rs`: logical TeX/TFM request normalization over `umber-vfs` canonical paths.
- `src/virtual_compile/pdf_resources.rs`: post-execution typed VF/local-TFM/map/encoding/program closure discovery and immutable parsed cache.
- `src/virtual_compile/resolvers.rs`: VFS-snapshot-backed input/font resolvers that register selected bytes through World, with typed missing-file and logical OpenType-font side state.
- `src/virtual_compile/tests.rs`: native retry, path, precedence, limits, format, effect-isolation, font batching, and DVI coverage.
- `src/main.rs`: `umber` binary entry point, CLI argument parsing, `lex-dump`/`expand-dump`/`run` dispatch, token formatting, and real-run file resolvers.
- `src/cli_resource.rs`: retained native project/cache/distribution resolution, cancellation-aware resource retries, incremental source replacement, finite engine-fuel configuration, and accepted-run telemetry handoff.
- `src/cli_resource/tests.rs`: retained-resource reuse and superseded-revision cancellation coverage.
- `src/watch.rs`: polling incremental watch driver, supersession/Ctrl-C cancellation, DVI publication, and phase latency reporting.
- `src/bin/gentle_profile.rs`: persistent optimized Gentle profiling runner with optional `profiling-stats` counters that preloads the external corpus into a shared in-memory World, isolates fresh cold sessions under explicit memo policies, and separately enforces slow pagination-changing, cross-generation interaction, fast suffix-adoption, and shared-mount hlist-rebreak paths under memo disabled/enabled or explicit baseline/candidate policies with cold-DVI, named-boundary-schedule, and profiling-only state-hash journal-work verification.
- `tests/it.rs`: integration-test module root wiring CLI, replay identity, effectful replay, and end-to-end conformance suites.
- `tests/it/cli.rs`: integration tests for CLI success, usage errors, corpus dump output, and committed diagnostic/DVI fixture parity.
- `tests/it/e2e_conformance.rs`: individually selectable Story, Gentle, TRIP, and e-TRIP tests that execute Umber in process against gitignored, locally generated `tests/corpus/e2e` DVI oracles through `parity-harness`; TRIP and e-TRIP share one two-phase format helper, and each case runs conditionally when its external inputs and oracle exist.
- `tests/it/effectful_replay.rs`: property tests for rollback and commit identity across terminal, log, stream, input, read, and shipout effects.
- `tests/it/pdf_parity.rs`: hermetic pinned-pdfTeX normalized structure, exact Umber byte, and Poppler raster-attestation fixture gate.
- `tests/it/replay_identity.rs`: property and regression tests that generated primitive programs rollback to identical state.

## Validation

Run `cargo test --tests -p umber` after CLI or composed-runner changes. For behavior that changes emitted diagnostics or fixtures, follow `tests/AGENTS.md` and regenerate deliberately with `scripts/regen-fixtures.sh`. Ordinary corpus tests consume committed fixtures; external end-to-end conformance tests conditionally consume locally generated oracles.
