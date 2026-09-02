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
- `src/engine_session.rs`: `EngineSession`, the retained host-neutral entry point for `tex-command`/`MainControl` execution; canonical bounded-episode resume and telemetry, explicit complete-job versus authored-fragment completion, resource-suspension fulfillment, checkpoint publication, and the `ResourceHost` contract used by real drivers and by `examples/first_failure_locator.rs`.
- `src/expand_dump.rs`: implementation of the `expand-dump` CLI command through the shared engine session and dump primitive setup.
- `src/format_cache.rs`: TeX-specific generated-format identity, schema and engine validation, opaque construction-evidence policy, and the adapter over `umber-fetch::BlobStore`, including old-layout migration.
- `src/format_cache/tests.rs`: format identity, validation, compatibility, recovery, and cross-process concurrency coverage.
- `src/format_cache_cli.rs`: pinned LaTeX/pdfLaTeX generated-format cache identity, validated restore, and atomic publication CLI adapter.
- `src/format_fixture.rs`: generic content-addressed format recipes, fresh loaded-universe reconstruction, typed resource fulfillment, and the raw TeX82 loaded-fixture first slice.
- `src/format_worker.rs`: explicit production/libtest earliest-entry launcher contract, stable-executable per-child authenticated recipe/result protocol, and killable native wall/RSS-supervised format-construction process.
- `src/prepared_format.rs`: universal native persistent prepared-format provider, explicit job request including job-local provenance demand, compatibility validation, and provider-owned fresh-memory World boundary.
- `src/prepared_format/tests.rs`: persistent provider preparation, cache recovery, compatibility, guard, and fresh-job isolation coverage.
- `src/linux_rss.rs`: checked runtime-page-size Linux resident-set conversion shared by cooperative and parent format guards.
- `src/format_fixture/tests.rs`: format identity, worker budget, failure atomicity, cache reload, concurrency, and loaded-execution coverage.
- `src/bib.rs`: native host-file staging, resource retry, and detached artifact publication for the in-process `bib` command.
- `src/classic_bib.rs`: native host-file staging and artifact publication for the in-process classic `bibtex` command.
- `src/input_search.rs`: deterministic driver-owned TeX input and TFM font path resolution through World-backed reads.
- `src/input_search/tests.rs`: focused TeX input/font area ordering, extension, and input-record coverage.
- `src/input_observation.rs`: versioned accepted input-dependency projection shared by native and WebAssembly sessions.
- `src/fixed_point.rs`: shared deterministic pass/attempt bounds and non-adjacent fixed-point oscillation policy.
- `src/editor_session.rs`: native provisional/stabilizing/stable editor coordinator over one-pass incremental and TeX fixed-point sessions.
- `src/editor_session/tests.rs`: editor stabilization state, revision identity, generated-file selection, no-op pass counts, cancellation, and rollback coverage.
- `src/latex_project.rs`: shared-workspace transactional TeX and optional bibliography multipass orchestration, canonical non-file admission, convergence, and atomic project acceptance.
- `src/latex_project/support.rs`: project candidate VFS assembly, generated-file identity, and shared resource conversion helpers.
- `src/latex_project/tests.rs`: project convergence, bibliography publication, and rollback coverage.
- `src/tex_fixed_point.rs`: public bibliography-free TeX fixed-point adapter over the shared project candidate machinery.
- `src/tex_fixed_point/tests.rs`: shared primitive/LaTeX-surface fixture convergence, cold identity, generated-input selection, resource resumption, bounds, oscillation, and rollback coverage.
- `src/lib.rs`: canonical retained-root run helpers, file resolvers, typed
  effect-before-driver finalization, and one-artifact-at-a-time DVI
  construction. Umber has no legacy lexer/gullet session adapter, including in
  tests.
- `src/memory_output.rs`: exact committed terminal/log/DVI/aux collection for successful memory-backed runs, aggregate output limits, and auxiliary publication into VFS generated transactions.
- `src/memory_output/tests.rs`: final-commit idempotence, output accounting, and memory-boundary tests.
- `src/pdf_import.rs`: lightweight PDF syntax inspection for host-side external-page request resolution; detached resource import belongs to `tex-out`.
- `src/pdf_import/tests.rs`: synthetic named-page inspection regressions.
- `src/pdftex.rs`: pdfTeX 1.40.29 behavior, focused conformance tests, and thin
  mode/default application over `tex-command`'s integrated primitive
  catalogue; it owns no second name, meaning, parameter, or default table.
- `src/pdftex/tests/retained_fixture_properties.rs`: active retained pdfTeX-extension fixture runner that compares status, terminal, and log projections, including bug-linked strict xfails.
- `src/pdf_output.rs`: thin detached-finalization adapter, error translation, diagnostics publication, and validated allocation-receipt replay; all lowering and serialization delegate to `tex-out`.
- `src/pdf_output/tests.rs`: detached boundary tests for container classification, host-resolved font resources, and exact nested virtual-font identity, sizing, rejection, and independent-parse evidence.
- `src/pdf_output/finalization_input.rs`: compatibility adapter that freezes
  accepted engine state and host-resolved artifacts/resources into
  `tex_out::pdf::PdfFinalizationInput`; it is the only Umber-owned PDF
  finalization boundary.
- `src/pdf_output/finalization_input/virtual_fonts.rs`: destination-local,
  handle-free discovery and atomic allocation of the sized font instances
  selected while detached virtual-font packets are lowered.
- `src/pdf_output/finalization_input/virtual_fonts/tests.rs`: unified pdfTeX
  engine/VF internal-font timeline regression coverage.
- `src/virtual_compile.rs`: host-neutral persistent compile session over one `ProjectWorkspace`, versioned mapped-TFM layout policy, revision-checked root patches, canonical file/OpenType/PK admission and retries, atomic response registration, one retained canonical HTML render document, output-budgeted rendered-source caches, retained immutable resources, independently configurable restart-history retention, and execution/resource accounting.
- `src/virtual_compile/path.rs`: logical TeX/TFM request normalization over `umber-vfs` canonical paths.
- `src/virtual_compile/pdf_resources.rs`: post-execution typed VF/local-TFM/map/encoding/program closure discovery and immutable parsed cache.
- `src/virtual_compile/resolvers.rs`: VFS-snapshot-backed input/font resolvers that register selected bytes through World, with typed missing-file and logical OpenType-font side state.
- `src/virtual_compile/tests.rs`: native retry, path, precedence, limits, format, effect-isolation, font batching, and DVI coverage.
- `src/main.rs`: `umber` binary entry point, CLI argument parsing, canonical `CommandState` source-tokenization for `lex-dump`, `expand-dump`/`run` dispatch, token formatting, profiling-only TeX82 memory-projection and machine-readable hot-core census publication (including failed bounded runs), accepted PDF font-closure receipt publication, and real-run file resolvers.
- `src/cli_resource.rs`: retained native project/cache/distribution resolution,
  the source/digest/offline-bounded authenticated root plus touched packed-shard
  byte owners, cancellation-aware resource retries, incremental source
  replacement, one-shot zero restart-history ownership independent of the
  resource cache, finite engine fuel/step/frame/journal/effect configuration,
  accepted-run telemetry handoff, and identity-pinned PDF font-closure receipt
  projection. Packed misses are authoritative; do not restore selected-record
  or selected-miss caches.
- `src/cli_resource/tests.rs`: retained-resource reuse and superseded-revision cancellation coverage.
- `src/distribution_verify.rs`: explicit pinned local root/shard/object graph verifier, streaming object authentication, and complete-work report.
- `src/distribution_verify/tests.rs`: exhaustive distribution verifier positive and corruption controls.
- `src/watch.rs`: polling incremental watch driver, bounded distribution-owner reuse across replacement sessions, same-thread non-atomic engine ownership with a separate file monitor, supersession/Ctrl-C cancellation, DVI publication, and phase latency reporting.
- `src/bin/distribution_startup_benchmark.rs`: hermetic cold-process versus same-process multi-session distribution benchmark with exact cache-inventory and DVI-loss assertions.
- `src/bin/distribution_verify.rs`: separately invoked complete local distribution and native cache integrity audit.
- `src/bin/gentle_profile.rs`: persistent optimized Gentle profiling runner with optional `profiling` counters that preloads the external corpus into a shared in-memory World, isolates fresh cold sessions under explicit memo policies, measures balanced unchanged-root generated-input stabilization replay and long-session retention, and separately enforces slow pagination-changing, cross-generation interaction, fast suffix-adoption, and shared-mount hlist-rebreak paths under memo disabled/enabled or explicit baseline/candidate policies with cold-DVI, named-boundary-schedule, and profiling-only state-hash journal-work verification. The retired expansion meaning-cache invalidation census is not part of its report.
- `tests/it.rs`: integration-test module root wiring CLI, replay identity, effectful replay, and end-to-end conformance suites.
- `tests/it/cli.rs`: integration tests for CLI success, usage errors, corpus dump output, channel-matched committed terminal diagnostic and DVI fixture parity, and metadata-driven closed native bibliography invocation cases.
- `tests/it/e2e_conformance.rs`: individually selectable Story, Gentle, TRIP, and e-TRIP tests that execute Umber in process against gitignored, locally generated `tests/corpus/e2e` DVI oracles through `parity-harness`. Every full-pipeline route prepares a complete Plain, raw TeX82, TRIP, or e-TRIP recipe through `PreparedFormatProvider`, then runs a fresh explicit `PreparedFormatJob`; no family helper owns a cache, worker, INITEX session, dump/load, image decode, staged Plain host, or mutable loaded universe. Story and Gentle share Plain, the seven self-contained DVI regressions share raw TeX82, and TRIP/e-TRIP retain distinct typed recipes. Definition-anchored controls enumerate all callers and preserve finite guards, authenticated construction evidence, exact loaded channels and normalized DVI, plus advisory geometry. Direct format image/security tests remain separate, and full-document gates do not broaden the automated microfixture tracer (see "Canonical Story and Gentle Regression Gates" in `docs/testing_infrastructure.md`). Failing compatibility/parity assertions excluded from the command-core cutover closure gate retain explicit `#[ignore]` reasons and run manually with `cargo test -q -p umber --test it -- --ignored`; passing behavioral and parity checks remain routine.
- `tests/it/e2e_conformance/assets.rs`: the registry and single reachability choke point for the four byte-exact e2e DVI gates. `GATES` names each gate's external inputs and materialization commands; `with_gate` runs a gate body or fails with an actionable absence report, and writes its confirmation and opt-out notices to the process's real stderr handle so libtest's output capture cannot hide them. Two meta-tests hold the registry in correspondence with `.gitignore` and with the real `with_gate` call sites. See "End-to-End Conformance Gate Contract" in `docs/testing_infrastructure.md`.
- `tests/it/e2e_conformance/etrip_official.rs`: supplemental official e-TeX V2 master-artifact comparison for the exact e-TeX 2.6 e-TRIP gate. It pins the minimal V2-to-2.6 text bridge, exact generated `etrip.out`, and a typed projection of the official DVItype master without invoking host TeXware.
- `tests/it/effectful_replay.rs`: property tests for rollback and commit identity across terminal, log, stream, input, read, and shipout effects.
- `tests/it/font_catalog.rs`: exact HTML MVP TFM/mapping, decoded WOFF2
  program, cmap, MATH, license, and retained-object inventory audit.
- `tests/it/pdf_parity.rs`: hermetic pinned-pdfTeX normalized structure, exact Umber byte, and Poppler raster-attestation fixture gate.
- `tests/it/replay_identity.rs`: property and regression tests that generated primitive programs rollback to identical state.
- `examples/first_failure_locator.rs`: reusable direct canonical Gentle/Story e2e first-failure locator (`umber2-johp.57`). Stages `third_party/corpus/{plain,<source>}.tex`, `third_party/hyphen/hyphen.tex`, and the plain-format CM/`manfnt` TFMs (via `parity_harness::{CORPUS_TFMS, locate_tfm}`) into an in-memory `World`, drives them through `EngineSession`, and on the first `ExecError`/panic reports the live mode, provenance-resolved TeX source context, and (for panics) lets the default panic hook report the Rust `file:line` origin. It can only show that execution stopped, never that completed output is wrong (see the Glossary in `docs/canonical_divergence_workflow.md`). Run with `cargo run -p umber --example first_failure_locator -- gentle` (or `story`); not part of `cargo test`. See `docs/tex_command_core.md` and the current open successor issue under the `umber2-johp` epic (`bd show umber2-johp` for its children) for the currently tracked Gentle divergence it reproduces.

## Validation

Run `cargo test --tests -p umber` after CLI or composed-runner changes. For behavior that changes emitted diagnostics or fixtures, follow `tests/AGENTS.md` and regenerate deliberately with `scripts/regen-fixtures.sh`. Ordinary corpus tests consume committed fixtures; external end-to-end conformance tests consume locally generated oracles and fail with materialization instructions when those are absent (see "End-to-End Conformance Gate Contract" in `docs/testing_infrastructure.md`).

Diagnosing a `umber2-johp` divergence has a fixed recipe: run the
differential tracer (`cargo run -q -p tex-command-stream --bin tex-command-stream -- --repository .`)
first, then the `first_failure_locator` example for the live end-to-end
front, and only fall back to manual instrumentation if both come up short.
See the diagnosis order in
[Canonical Divergence Working Contract](../../docs/canonical_divergence_workflow.md#2-diagnosis-order)
for why ad hoc probes and the retired Umber engine are excluded, and
[Testing Infrastructure](../../docs/testing_infrastructure.md) for each
tool's exact commands and output shapes.
