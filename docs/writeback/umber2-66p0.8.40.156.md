# `umber2-66p0.8.40.156`: resident command-visible state

## Adopted ownership boundary

`Universe` now owns the actual command-visible stores in one resident
`CommandVisibleState`. The state is neither an alias nor a cache: format
materialization, checkpoint fork/settlement, rollback, hashing, retirement,
and ordinary Universe APIs all address those same owners. `CommandContext`
borrows the resident owner directly and adds only the independently admitted
dense core plus the checked page-node/page-builder split required by
`PageRegionHistory`.

The deleted `CommandContextParts` assembled nineteen references or values on
every admission. The replacement constructor receives four authoritative
borrows. Rust still prevents a context from surviving its Universe borrow or
overlapping checkpoint, suspension, rollback, and publication mutation. No
unsafe code, persistent alias, copied semantic state, policy driver, or
command-family path was added.

## Focused comparison

The exact baseline was built from assigned base `655c56ff0`; its binary
SHA-256 is
`66f2e2c217d7751e431c703968643f8f50b5cc4d43e47003f6d02e3691e7302d`.
The candidate binary SHA-256 is
`9732cb8b42b2bf54e47720bee44d7bd663a9e9765dcb5d69453f604aa9ce0a86`.
One matched profiling pair ran `canonical_episode 1000 10 direct`. Both sides
reported the exact work vector: 67,083 fuel charges, 65,083 raw frame steps,
46,019 expanded deliveries, 20,035 meaning lookups, 2,000 nodes, 19,196
artifact bytes, and 2,932 DVI bytes.

| Exact result                            |           Baseline |          Candidate |               Delta |
| --------------------------------------- | -----------------: | -----------------: | ------------------: |
| Seven `with_command_context` monomorphs |       23,812 bytes |       20,452 bytes |    -3,360 (-14.11%) |
| Complete executable `.text`             |    6,879,176 bytes |    6,885,197 bytes |     +6,021 (+0.09%) |
| User instructions                       |        302,577,361 |        301,495,499 | -1,081,862 (-0.36%) |
| User branches                           |         64,231,187 |         64,150,355 |    -80,832 (-0.13%) |
| User cycles                             |        152,579,147 |        155,782,481 | +3,203,334 (+2.10%) |
| Allocations / requested bytes           |  5,050 / 5,523,259 |  5,050 / 5,523,259 |           unchanged |
| Public `memcpy` calls / bytes           | 47,511 / 4,206,963 | 46,504 / 4,022,779 |   -1,007 / -184,184 |
| Public `memmove` calls / bytes          |            8 / 112 |            8 / 112 |           unchanged |

The deterministic instruction, selected-symbol, and copy reductions are the
acceptance evidence. The single-pair cycle increase is recorded and is not a
speed claim. Copy reports reconcile exactly with zero overflow or
probe-internal calls. The removed 1,000-call, 168,000-byte copy row matches the
per-application admission frequency; the remaining 7 calls and 16,184 bytes
of reduction are cold layout/code-generation effects.

The baseline/candidate `perf stat` receipt hashes are
`d7bfa4338a1774876b39c3a338b49983a2c6aae2f284b1f6e3640512bb071581` and
`87da0f2833a85e6f06751d45eed8b99683749257430ea4b0b3c2c8d330c0f72f`.
The symbolized copy-report hashes are
`f2ef123e67cec210d8010d680c749bf52a22de263bb304ae090f0942d4aa6e66` and
`9c79f6b86687d7072276a6ed7abf35a908d01bb944ed70b3e666c48154a07163`.
The checked interposer SHA-256 is
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.
Ignored evidence is under `target/umber2-66p0.8.40.156/`.

## Validation

- `cargo test -q --tests -p tex-state -p tex-command -p tex-exec`: 2,390
  tests pass; two existing tests are ignored.
- `scripts/check.sh`: all four gates pass.
