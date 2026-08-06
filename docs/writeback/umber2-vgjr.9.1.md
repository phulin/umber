# umber2-vgjr.9.1 — canonical primitive descriptor schema

Authority: [`tex-command`'s primitive catalogue schema](../../crates/tex-command/src/primitives/catalogue.rs), based on source snapshot `74d21b41e`.

The new dependency-light descriptor vocabulary gives every catalogue row an
explicit stable operand namespace and value, profile membership, canonical or
alias spellings, expansion class, profile-specific WEB `cmd`/`chr` identity,
prefix admissibility, per-spelling fresh-INITEX/format-registry policy,
parameter cell and allocation-independent default, and documentation family.
It contains no execution callback, dispatch function, or Rust-enum ordinal.

The operand domains cover both enum-backed commands and every current
exception shape: internal integers, the five parameter banks, page dimensions
and integers, font selectors such as `nullfont`, `relax`, frozen macros such as
`endwrite`, and inaccessible commands. Punctuation, control space, aliases,
and profile-specific spellings use the same spelling record. Job-clock and
glue defaults are semantic values rather than host or allocation identities.

`PrimitiveCatalogue::validate` returns the complete error set. It rejects
empty or out-of-profile records, missing canonical spellings, operand,
spelling, WEB-identity, and parameter-cell collisions, parameter operand/cell
mismatches, and default/domain mismatches. Intentional shared WEB identities
require an explicit common collision group, so exceptions cannot silently
weaken uniqueness checking. Focused tests cover every current operand domain,
alias/default/frozen representation, every uniqueness namespace, explicit WEB
equivalence, and simultaneous structural failures.

This issue defines and exports the schema only. `umber2-vgjr.9.2` owns
generation of registry, observation, policy, default, and documentation views;
`umber2-vgjr.9.3` owns consumer migration and deletion of the existing tables.
No behavior or predecessor inventory changed here.

Validation on the final tree was serialized as required:

- `cargo test -q -p tex-command --lib --no-run`, then the same scope under
  `systemd-run --user --scope -p MemoryMax=512M` and `timeout 300s`: 678
  passed, 0 failed;
- `cargo test -q --tests --no-run`, then the full workspace under
  `systemd-run --user --scope -p MemoryMax=1G` and `timeout 1200s`: all test
  binaries passed; and
- `scripts/check.sh` under the same 1 GiB cgroup and 1,200-second timeout: all
  four gates passed, with both lint passes clean across 28 workspace members.

Before this writeback, implementation metrics were 733 additions and 1
deletion: 717 lines in the schema and focused tests, plus 16 additions and 1
deletion in exports and crate guidance. This is foundation work; net deletion
is intentionally deferred to the two consuming children named above.
