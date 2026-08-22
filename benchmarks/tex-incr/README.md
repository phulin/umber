# tex-incr benchmarks

This standalone crate contains focused two-generation accepted/rejected edit
diagnostics for the incremental owner and is excluded from the root workspace
correctness gate.

Run it with:

```bash
cargo bench --manifest-path benchmarks/tex-incr/Cargo.toml \
  --bench accepted_edit
```

`accepted_edit` uses a real incremental session and verifies the initial cold
run before timing acceptance and rejection separately. Acceptance must retain
exactly current and retire prior; rejection drops current while prior remains
the sole retained generation.
