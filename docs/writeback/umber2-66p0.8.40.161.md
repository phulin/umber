# `umber2-66p0.8.40.161`: direct admitted arena block operations

## Adopted path

An admitted list cursor now retains the stable `u32` vector coordinates of
its current typed 64-KiB block and its physical base. Root admission resolves
the head and tail blocks after the existing pool, owner, logical-incarnation,
physical-incarnation, initialized-prefix, and endpoint checks. A predecessor
transition resolves the next block once. Values within that block use only
the retained block coordinate plus their offset.

The cursor remains 16 bytes, matching the former owner-relative cursor. Long
forward walks therefore retain the direct coordinate without enlarging the
recursive predecessor-walk frame. It stores no pointer, reference, physical
identity in a published root, successor table, cache, or alternate payload
representation. The immutable pool borrow excludes relocation and lifecycle
mutation while an `ArenaListView` exists. The mutation-compatible page-list
cursor keeps its prior copied logical coordinate and still rejects a suffix
which an operation rollback recycled.

Unsealed typed-annex publication now admits one append block per list or
physical boundary. Its scalar continuation holds the logical key, physical
block index, initialized index, and logical offset. Values then append
directly to that block and advance the initialized prefix until it fills.
Allocation, tail reuse, owner/lineage/incarnation validation, predecessor
publication, and rollback remain at the existing transition boundaries.
Identity-bearing, dependency-bearing, destination construction, and semantic
copy paths keep their distinct checked transactions; this is not a node-only
fast path.

## Focused evidence

The committed `arena_block_ops_gate` represents both relevant shapes:

- 4,096 independently addressed 11-word lists, matching the fixed
  `ListPayload` annex width and the ordinary repeated unsealed-list path;
- one 65,536-word list crossing four annex blocks; and
- 64 read passes through both `ArenaListView` compatibility iteration and the
  mutation-compatible admitted chunk cursor.

The exact-base binary is commit `a9ffd010c` plus only the identical
testing-feature gate seams and benchmark. The candidate is the direct-block
implementation before the separate inherited-tip rustfmt-only cleanup. Both
produce checksum `202396794880`, seven logical blocks, zero warmed
allocation calls, and zero warmed requested bytes. Seven-run `perf stat`
means over `cycles:u,instructions:u` are:

| Counter      |  Exact base |   Candidate |                 Change |
| ------------ | ----------: | ----------: | ---------------------: |
| Instructions | 728,020,544 | 341,395,795 | -386,624,749 (-53.11%) |
| Cycles       | 205,388,796 | 107,310,898 |  -98,077,898 (-47.75%) |

The checked public-copy interposer reports the same whole-process totals for
both binaries: 18 `memcpy` calls / 153 bytes and two zero-byte `memmove`
calls. Both tables have zero overflow and zero probe-internal calls. The hot
arena workload therefore adds neither a public `memcpy` nor a public
`memmove`; the identical small totals are process startup and reporting.

The exact-base and candidate binary SHA-256 values are respectively
`4fde488adf24b1fa8f6fcfafdd1405356df46b517472fb4dc37d70e9361f52d2`
and
`8d6f6a8ab72afc9adf6285042cf67c688ac2ff579f53b888f23dbe4dd5f32a04`.
The interposer SHA-256 is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Ignored raw copy reports are under
`target/umber2-66p0.8.40.161/evidence/`; their base and candidate SHA-256
values are
`54a577af0278b1e7789fcf123f865de49c20938fae5f2c89203aab6e4ba8b247`
and
`eb9416c323796634f598e9f47b9e6904202625e9a976fda771abeabae4265c72`.

## Correctness and lifecycle evidence

`unsealed_sequential_append_admits_only_at_packed_block_boundaries` warms and
rolls back 4,096 packed words, then requires zero allocation and exactly three
validation reads per crossed block rather than per value. Existing exact
tests retain endpoint admission, direct and indexed traversal parity,
allocation-free chunk traversal, stale rollback rejection, shared-prefix
append, accepted/candidate settlement, and node-plus-annex rollback coverage.

Validation results:

- `cargo test -q --tests -p tex-state`: 578 unit tests, 12 boundary tests, and
  one compile-fail boundary test pass;
- `cargo check -q -p umber-wasm --target wasm32-unknown-unknown` passes, with
  only the existing unused `set_checkpoint_budget` warning; and
- `scripts/check.sh`: all four gates pass; both Clippy resolutions are clean
  across 32 workspace members.
