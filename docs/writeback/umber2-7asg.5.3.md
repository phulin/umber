# `umber2-7asg.5.3`: Borrowed current-command meaning resolution

## Boundary and implementation

Commit `647680943` exposes a borrow-scoped direct `DenseBank` row read through
`DenseState::meaning_word` and
`CommandContext::compact_control_sequence_meaning_word`. The existing owned
`compact_control_sequence_meaning` handoff now resolves from that borrowed row.
Static meanings decode in place; a macro `DefinitionRef<G>` is cloned once into
the owned `ResolvedMeaning` retained by the final `CurrentCommand`. Frozen
primitive resolution likewise resolves from its borrowed slice row instead of
cloning a complete `MeaningWord` first.

The borrow ends before `resolve_into` returns. No borrow, state coordinate,
assignment level, or journal word enters `CurrentCommand`. Once-at-delivery
meaning, delivery stamps, provenance, alignment adjustment, outer recovery,
TeX local/global restoration, operation rollback, replay, retry, suspension,
and generation branding retain their existing ownership and ordering. The
change adds no cache, lazy lookup, token special path, heap indirection, or
duplicate semantic table, and leaves the destination-directed caller handoff
for `umber2-7asg.5.4` unchanged.

Focused state coverage proves that borrowing a macro row leaves its semantic
owner count at two, resolving the final owned value raises it to three exactly
once, and dropping that value restores two. Command coverage reassigns the
meaning cell and drops the caller's original owner after delivery, then reads
the replacement word through the still-owned delivered command.

## Exact 20M measurement

The optimized frame-pointer binary has SHA-256
`69aadc5118aac0e1e8555017f785e5df7bb2a24a231384d18bd161cd81969ef4`.
It reused the immutable source, authenticated distribution and schema-12 format,
123-key prefetch closure, offline policy, fixed clock, 20M fuel, and guards from
[`umber2-7asg.5.2`](umber2-7asg.5.2.md). Every host row was serialized with
`flock /tmp/umber-perf-host.lock` and used an issue-private cache and output
directory.

Both controls and the perf row intentionally exited 1 at exact fuel exhaustion
and reproduced `(20000000,19913119,2218327,6020965,16785710,4011)` exactly.
The warmed controls were 8.22 and 8.47 seconds wall, 9.12 and 9.16 seconds user,
and 327,176 and 327,076 KiB peak RSS. The perf row was 9.17 seconds wall and
captured 1,633 samples with zero lost samples. Its exact period sum is
19,380,728,427 weighted user cycles.

| `CurrentCommand::resolve_into` measure | `.5.2` baseline |  Borrowed row | Absolute change | Relative change |
| -------------------------------------- | --------------: | ------------: | --------------: | --------------: |
| weighted self cycles                   |     886,122,219 | 1,013,455,502 |    +127,333,283 |         +14.37% |
| weighted inclusive cycles              |   1,031,349,285 | 1,172,956,898 |    +141,607,613 |         +13.73% |
| self cycles per completed raw frame    |           44.50 |         50.89 |           +6.39 |         +14.37% |
| inclusive cycles per completed frame   |           51.79 |         58.90 |           +7.11 |         +13.73% |

The before/after assembly gives the ownership result independently of sampling.
For each ordinary macro row, the baseline performs two owner increments and
balances the temporary row owner with one release; the accepted binary performs
one `incq` for the final command owner and no temporary-row release. The
destination overwrite guard's unreachable-on-entry release remains in both
binaries and is not a meaning-row owner operation. Thus macro delivery removes
two owner-count read-modify-write operations across the exact 2,610,646 macro
resolutions while preserving the required final owner.

The zero-loss cycle capture does not show a resolution speedup: it records the
absolute regression above even though the redundant ownership instructions are
absent. The bounded slice is therefore an ownership foundation, not a claimed
cycle reduction. Subsequent delivery work must use these absolute figures as
its before state rather than attributing an inferred gain to this change.

## Allocation and validation evidence

The warmed packed cutover gate reports zero allocations and zero requested
bytes for ordinary source delivery, packed backup/replay, stored-token replay,
stored control-sequence delivery, and macro matching/replay/expansion. It ends
with `packed token/macro cutover gate: PASS`. Its receipt has SHA-256
`79801a4eb1b67f92ae608b5fa6ec61a8246ab732c9c090670fc04c6fec26ccc3`.

The local evidence is under `target/umber2-7asg.5.3/`; the accepted perf data,
raw events, and self report have SHA-256 values
`5c0410571460f8529aee8123fb75513acea995c448115502e676707d1ad8b09d`,
`79e2df7fc08a352528e1a04464028e53c784ee0190f1192d082ab60851d3a0c6`,
and `36b48c2bb2d67ed37f75060a01343393bb9cec4d3be996b71e3cb5743a49fdd3`.
Focused `tex-state` and `tex-command` suites pass. The complete routine suite
passes, and `scripts/check.sh` reports all four gates passed.
