# Semantic hashing model

This standalone crate compares two complete semantic-hashing pipelines for a
TeX-like workload:

- `CurrentSystem` keeps compact interned symbols and resolves their canonical
  kind/name while traversing every checkpoint-rooted token list.
- `PromotedSystem` additionally computes one canonical semantic atom per
  interned symbol and lazily promotes checkpoint-rooted token lists to fixed
  semantic identities.

The deterministic workload contains macro-sized and token-register-sized
lists, repeated primitive and user control sequences, character runs,
parameter tokens, hot roots reused across paragraph boundaries, and cold roots
introduced throughout a session. Tests rebuild the same semantic workload with
different symbol allocation orders.

Run:

```bash
cargo bench --manifest-path benchmarks/semantic-hash-model/Cargo.toml
```
