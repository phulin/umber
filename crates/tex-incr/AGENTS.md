# tex-incr Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
long-lived editor-session strategy over executor-named checkpoints.

## Boundaries

- Treat `EngineCheckpoint` and `CheckpointRetention` as opaque aggregate roots.
- Restart only through the checkpoint's tex-command-owned summary and explicit
  executor/runtime roots; do not recreate lexer or expander state.
- Never traverse `tex-state` substores or manufacture checkpoint boundaries.
- Correctness is byte-identical accepted artifacts/DVI versus a cold run; reuse
  is optional when schedule, anchor, or state-hash validation fails.
- Accepted history must name one revision directly and must not retain revision-map chains.

## File Map

- `Cargo.toml`: incremental driver dependencies and workspace lint policy.
- `src/history.rs`: handle-free named-boundary observations, convergence
  comparison, and prune-first durable-root selection policy.
- `src/lib.rs`: revision/edit model, host-supplied resolver execution,
  immutable resource retry overlays, rendered-source demand selection and lazy
  budgeted/evictable artifact-root/recipe queries, terminal recognition of
  complete-job and explicit-fragment outcomes, candidate acceptance, and
  detached accepted output views, and opaque coarse generation/checkpoint
  ownership whose runtime coordinates remain inside generic admission.
- `src/trace.rs`: derived ordered leaf/parent trace summaries, dependency reduction, and atomic replay.
- `src/trace/tests.rs`: parent composition, leaf-equivalence, ordering, and atomic-miss coverage.
- `src/tests.rs`: synthetic edit, convergence, retention, and cold-parity tests.
- `src/tests/long_session.rs`: routine accepted/rejected revision, resource
  retry, checkpoint, effect, artifact, and cold-DVI equivalence coverage plus
  the explicit 2,048-cycle semantic stress tier.

## Validation

Run `cargo test --tests -p tex-incr`. When changing edit mapping or convergence,
run the explicit 1,000-edit tier with
`cargo test --tests -p tex-incr tests::thousand_edit_scripted_fuzz_matches_cold_every_revision -- --ignored --exact`
as well. Long-session semantic changes also run
`cargo test -q -j 1 --tests -p tex-incr tests::long_session::long_session_thousands_match_clean_at_equal_work_milestones -- --ignored --exact --test-threads=1`.
