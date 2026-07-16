# tex-incr Guidance

Read the repository-level `AGENTS.md` before editing here. This crate owns the
long-lived editor-session strategy over executor-named checkpoints.

## Boundaries

- Treat `EngineCheckpoint` and `CheckpointRetention` as opaque aggregate roots.
- Never traverse `tex-state` substores or manufacture checkpoint boundaries.
- Correctness is byte-identical accepted artifacts/DVI versus a cold run; reuse
  is optional when schedule, anchor, or state-hash validation fails.
- Accepted history must name one revision directly and must not retain revision-map chains.

## File Map

- `Cargo.toml`: incremental driver dependencies and workspace lint policy.
- `src/lib.rs`: revision/edit model, host-supplied resolver execution, immutable resource retry overlays, named-boundary history, pruning, convergence, and non-consuming accepted output views.
- `src/delivery.rs`: stable source, macro, token-list, and synthetic trace-delivery identities.
- `src/delivery/tests.rs`: delivery-identity occurrence and content-sharing coverage.
- `src/episode.rs`: memo-owned transient token episodes and explicit one-time durable publication.
- `src/trace.rs`: derived ordered leaf/parent trace summaries, dependency reduction, and atomic replay.
- `src/trace/tests.rs`: parent composition, leaf-equivalence, ordering, and atomic-miss coverage.
- `src/tests.rs`: synthetic edit, convergence, retention, and cold-parity tests.

## Validation

Run `cargo test --tests -p tex-incr`; run the scripted fuzz tier through
Run the explicit 1,000-edit tier with
`cargo test --tests -p tex-incr tests::thousand_edit_scripted_fuzz_matches_cold_every_revision -- --ignored --exact`
when changing edit mapping or convergence.
