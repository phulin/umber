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
- `src/lib.rs`: revision/edit model, named-boundary history, pruning, convergence, and accepted output.
- `src/tests.rs`: synthetic edit, convergence, retention, and cold-parity tests.

## Validation

Run `cargo test --tests -p tex-incr`; run the scripted fuzz tier through
`scripts/test-incremental-fuzz.sh` when changing edit mapping or convergence.
