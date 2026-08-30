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
- `src/candidate_lease.rs` and `src/candidate_lease/tests.rs`: move-only,
  zero-allocation current-candidate lease over the session-owned exclusive
  slot and its repeated claim/release high-water control.
- `src/history.rs`: handle-free named-boundary observations, convergence
  comparison, and prune-first durable-root selection policy.
- `src/lib.rs`: caller-owned reachability-store constructor and lifetime-bound
  session/revision APIs, revision/edit model, host-supplied resolver execution,
  immutable resource retry overlays, rendered-source demand selection and lazy
  budgeted/evictable artifact-root/recipe queries, terminal recognition of
  complete-job and explicit-fragment outcomes, candidate acceptance, and
  detached accepted output views, and opaque coarse generation/checkpoint
  ownership whose runtime coordinates remain inside generic admission,
  publication-time history-budget release propagated synchronously to every
  private checkpoint owner, plus the independently charged frozen JobStart
  image and its explicit profile/compatibility/job-clock binding. Command fuel
  consumption is read only when a candidate completes or exits with an error;
  ordinary executor steps and retained resource suspension keep the singular
  command ledger without cross-layer consumed-fuel publication. One
  external session reachability store owns the fixed prior/current physical
  slots across rejection, acceptance, and suspension; the admitted generation
  sidecar, not the revision runtime or retained checkpoint, owns the singular
  output chunk pool.
- Candidate driving holds the current generation in an owned aggregate guard;
  an attached-control guard restores temporarily detached runtime/control
  owners before unwind rejection, and prepared settlement rejects mode,
  command, boundary, ledger, state, page, and PDF ownership in dependency
  order from `Drop` unless Session consumes acceptance explicitly.
- `src/trace.rs`: derived ordered leaf/parent trace summaries, dependency reduction, and atomic replay.
- `src/trace/tests.rs`: parent composition, leaf-equivalence, ordering, and atomic-miss coverage.
- `src/tests.rs`: synthetic edit, convergence, retention, and cold-parity tests.
  It includes a caught-panic production regression at the exact detached-owner
  interval and proves prior-boundary sibling reuse after aggregate rejection.
- `src/tests/long_session.rs`: routine accepted/rejected revision, resource
  retry, checkpoint, effect, artifact, and cold-DVI equivalence coverage plus
  the explicit 2,048-cycle semantic stress tier.

## Validation

Run `cargo test --tests -p tex-incr`. When changing edit mapping or convergence,
run the explicit 1,000-edit tier with
`cargo test --tests -p tex-incr tests::thousand_edit_scripted_fuzz_matches_cold_every_revision -- --ignored --exact`
as well. Long-session semantic changes also run
`cargo test -q -j 1 --tests -p tex-incr tests::long_session::long_session_thousands_match_clean_at_equal_work_milestones -- --ignored --exact --test-threads=1`.
