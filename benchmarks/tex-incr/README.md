# tex-incr benchmarks

This standalone crate contains the focused accepted-edit diagnostic for the
incremental owner and is excluded from the root workspace correctness gate.

Run it with:

```bash
cargo bench --manifest-path benchmarks/tex-incr/Cargo.toml \
  --bench pure_memo_edit
```

`pure_memo_edit` compares an accepted edit with the bounded pretolerance,
page-breaking, and shipout memo runtime disabled and enabled. It uses a real
incremental session and verifies the initial cold run before timing the edit.
It measures the retained pure caches, not the deleted paragraph replay design.
