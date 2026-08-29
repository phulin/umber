# tex-incr benchmarks

This standalone crate contains focused two-generation accepted/rejected edit
diagnostics for the incremental owner and is excluded from the root workspace
correctness gate.

Run it with:

```bash
cargo bench --manifest-path benchmarks/tex-incr/Cargo.toml \
  --bench accepted_edit

cargo run --manifest-path benchmarks/tex-incr/Cargo.toml \
  --bin candidate-settlement-gate
```

`accepted_edit` uses a real incremental session and verifies the initial cold
run before timing acceptance and rejection separately. Acceptance must retain
exactly current and retire prior; rejection drops current while prior remains
the sole retained generation.

`candidate-settlement-gate` prepares real retained non-JobStart candidates,
then runs the production `Session` accept and transaction-reject paths. The
MainControl-to-prepared disposition scopes must report zero allocation calls
and requested bytes, while page-material counters across the complete
transitions must report zero source-node copies. PageBuilder settlement
counters additionally require zero checkpoint-capture scans, acceptance
payload scans, canonical-lane scans, and canonical-value copies.
