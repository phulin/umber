# `umber2-66p0.8.40.146`: one resident delivery tail

## Adopted boundary

`CommandState::advance_resident_command_into` still reads and discriminates the
authoritative top input row once. Its source, replay, attempt, durable,
macro-body, and macro-argument arms retain their existing storage-domain cursor
owners and first-touch journals. Each ordinary arm now ends its borrow with
only the packed word, origin, input identity/position, and source-policy
scalars. One branch-independent `EmptyCommand::write_resolved_delivery` call
then writes and resolves the caller-owned command, followed by the existing
single `settle_resident_delivery` tail.

The source helper no longer admits or resolves a command. Stored-token,
macro-body, and macro-argument arms no longer own a final-slot resolution or
settlement return. `OutParameter` detection remains before admission and
restarts the same macro loop after pushing the established parameter cursor.
Exhaustion retains each prior cold or resident retirement path. No carrier
struct, second loop, cache, storage redispatch, dynamic dispatch, cursor, or
command representation was added.

The external boundary gate now requires exactly one resident
`write_resolved_delivery` call and one settlement return, rejects a revived
stored-word delivery macro or aggregate carrier, and continues to require each
of the six concrete row arms exactly once.

## Focused mixed-resident evidence

The exact baseline executable was built from the issue base before editing;
the final executable was rebuilt from the completed source with the same
release/profiling manifest and toolchain. Both ran
`mixed_macro_resident_pipeline`, which reported 2,000,000 macro-body
transitions, 1,000,000 parameter deliveries, 1,000,004 replay words, 2,000,004
raw frame steps, 1,000,000 expanded deliveries, 1,000,001 macro expansions,
zero suspension moves, zero command copies, and zero warmed allocations or
requested bytes.

| Exact result                         |      Baseline |         Final |                Delta |
| ------------------------------------ | ------------: | ------------: | -------------------: |
| Resident profiling monomorph         |  10,833 bytes |   5,178 bytes |     -5,655 (-52.20%) |
| User instructions                    | 2,352,740,172 | 2,370,740,225 | +18,000,053 (+0.77%) |
| User branches                        |   388,192,350 |   389,192,505 |  +1,000,155 (+0.26%) |
| User cycles                          |   911,610,709 |   974,448,491 | +62,837,782 (+6.89%) |
| Internal elapsed nanoseconds         |   356,871,674 |   380,556,380 | +23,684,706 (+6.64%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |            unchanged |
| Public `memcpy` calls / bytes        | 142 / 353,154 | 142 / 353,151 |               0 / -3 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |            unchanged |

The material result is the 52.20% resident code-footprint reduction targeted by
the surviving 10,985-byte profile owner. Retired instructions, cycles, and the
short internal timer did not improve, so they are recorded explicitly and are
not used as speed claims. The code-size result removes duplicated admissions
without changing the exact work vector. Public-copy reports reconcile both
APIs with zero collisions, overflow, or probe-internal calls; neither report
attributes a copy to resident command delivery.

The checked public-copy interposer SHA-256 is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Baseline and final binary SHA-256 values are respectively
`8b6145f7358b0240355778add25643c46d39e4aecf9dffc7089a15bc3b90ae82`
and
`fb6f0efa035e2d50cdd699fdbe2d1f0cc182e92968a1d893fd83ef91f8ab1cec`.
Their `perf stat` receipt hashes are
`8a563c3a2c85bd73ebe6723fc9a67428685d417548e8a1fa269ae317d860b2a2`
and
`1de0e09683b75f75750d830a7181357cd99b347b513b1266f367c3dc84e5ed7d`;
their symbolized public-copy report hashes are
`90f650b095f24e3fe78afe61fe9ba482ed58e3633bdd9d858d7a6f5edba8637c`
and
`53ef42c630a11d03dcccbed0f600d7251b09c631440dacd963c936b27a780647`.
Ignored evidence is under
`target/umber2-66p0.8.40.146/focused-gate/`.

## Semantic coverage and validation

The command-core tests cover source, replay, attempt, durable, macro-body, and
macro-argument delivery through raw and expanded entries; exact source
position, origin, and role; parameter insertion and literal argument replay;
ordinary, terminal, source, macro, replay, and v-template end transitions;
outer/runaway/EOF recovery; typed resource suspension/resumption; and source,
resident-frame, macro-argument, replay, and operation rollback. The mixed
profiling fixture additionally proves the exact delivery vector, singular
transition, zero result redispatch, zero whole-frame/command copies, and zero
warmed allocation.

- `cargo test -q --tests -p tex-command`: 391 unit and 23 boundary tests pass.
- `cargo test -q --tests -p tex-command --features profiling`: 430 unit and 23
  boundary tests pass.
- `cargo test -q --tests`: complete routine workspace suite passes.
- `scripts/check.sh`: all format and Clippy gates pass.
