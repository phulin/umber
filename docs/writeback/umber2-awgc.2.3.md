# umber2-awgc.2.3: Fixed-Size HotCore Snapshots

## Outcome

Commit `e7bff53c2` publishes the storage-only HotCore aggregate required by the
third serialized substrate child. It composes the arena, stack, dense-bank,
inverse-journal, and external-cursor primitives delivered by `umber2-awgc.2.1`
and `.2.2`; it does not migrate MainControl, command scanning, formats, or
incremental checkpoint semantics.

`HotSnapshot` is a 192-byte copy-only runtime mark with zero owned retained
bytes. It contains only candidate and accepted-base identities, four arena
watermarks, six stack lengths, one typed mutation-journal cursor, and six
external-ledger cursors. Its type derives no serialization and is not exposed
through a format, memo, or detached checkpoint value.

## Atomic lifecycle

An aggregate restore preflights every identity, inverse record, arena mark,
stack mark, and external cursor before it mutates the first component. Valid
rollback restores dense values backward, truncates only the post-mark arena and
stack suffixes, and restores the external cursors. Stale, foreign,
cross-generation, and non-ancestor snapshots reject without partial state
change.

Accepted immutable arena layers remain readable through sibling candidates.
Candidate acceptance seals only its arena overlays; rejection drops the
candidate-local suffix; narrow transaction rollback permits retry against the
same accepted base while reusing warmed payload, stack, and journal capacity.
Nested commit transfers dense first-write ownership to the parent mark, so an
outer rollback still restores the original value.

## Durable controls

Crate-local tests in `crates/tex-state/src/hot_core/snapshot/tests.rs` pin the
mark layout and zero retention, constant shallow representation across empty
and large live state, accepted-base visibility, exact composite rollback,
nested commit, atomic stale/foreign rejection, cross-generation rejection,
exact all-live logical accounting, and exact storage plateau across 10,000
accept/reject/retry cycles.

The standalone `benchmarks/hot-core-snapshot` crate keeps live runtime handles
private behind a scalar testing facade. Its assertion-bearing command warms
every storage family and then executes 10,000 mixed cycles under a counting
allocator. The accepted result is zero allocation calls, zero requested bytes,
a 192-byte mark, zero snapshot-retained bytes, and final aggregate accounting
identical to the warm plateau. Criterion rows compare mark latency at zero,
1,024, and 65,536 live words and retain an all-family bounded rollback row.

## Validation

The following completed successfully on 2026-08-18:

- `cargo test -q --tests -p tex-state hot_core`: 35 focused tests passed.
- `cargo run -q --manifest-path benchmarks/hot-core-snapshot/Cargo.toml`:
  10,000 cycles, zero allocation calls, zero requested bytes.
- `cargo bench -q --manifest-path benchmarks/hot-core-snapshot/Cargo.toml --bench snapshots --no-run`:
  Criterion targets compiled.
- standalone benchmark rustfmt and all-target clippy with warnings denied.
- `cargo test -q --tests`: the complete routine workspace suite passed.
- `scripts/check.sh`: dprint, Biome, rustfmt, and both declared clippy
  resolutions passed.

This is necessary structural evidence, not a claim that the current
MainControl clone scopes have already disappeared. Packed input/macro adoption
and narrow command transaction migration remain owned by `umber2-awgc.3` and
`.4` respectively.

## Discovered work

The new allocation gate exposed that the pre-existing
`benchmarks/tex-state/src/lib.rs` no longer compiles against three removed
`Universe` testing methods. Issue `umber2-awgc.10` records that independent
repair and is linked to this discovery; no unrelated benchmark API repair is
part of this child.
