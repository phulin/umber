# `umber2-66p0.8.40.117`: resident cold command admission

## Selected exact owner

The exact caller-resolved authority from `umber2-66p0.8.40.110` ranks the
largest remaining `tex-exec` or `tex-command` owner outside every ForkedArena
family as `Result::expect -> MainControl::execute_cold_episode` at
`crates/tex-exec/src/main_control.rs:7365`: 564,210 public `memcpy` calls and
135,410,400 bytes. Every call is exactly 240 bytes.

The copied value was `CommandContext`, a borrow-only facade containing the
already-resident Universe owners as mutable references plus copy-small scalar
configuration. TeX state, the prepared cold operation, rollback roots,
suspension state, and provenance did not move with it. Returning that reference
aggregate from `Universe::command_context` through `Result::expect` was
therefore an ABI ownership transfer, not a TeX ownership requirement.

## Ownership change

Ordinary cold inspection, semantic application, named-token receipt drainage,
and settlement-fact capture now run inside `Universe::with_command_context`.
That existing admission boundary constructs the facade directly in the
callback's stack slot. The prepared operation remains in its caller-owned
`ColdOperationSlot` and is borrowed in place throughout. Only the existing
copy-small `PostApplyFacts`, redundancy bit, and detached observation leave the
callback. Immediate PDF-form admission uses the same construction rule before
its explicit host-publication boundary.

The e-TeX glue-pointer comparison now receives the two resident pointer-source
slices explicitly. This permits disjoint field borrowing while the command
machine is live; it does not change pointer identity or assignment behavior.
No cache, threshold, unsafe code, allocation, per-value owner, or alternate
execution path was added. Preparation, retry, rollback, suspension, provenance,
and exact command ordering remain on their prior owners and boundaries.

## Focused exact-copy gate

The existing `benchmarks/tex-exec` canonical episode ran 4,096 direct macro
calls under the checked caller-resolved public-copy interposer before and after
the change. Both runs produced the exact command vector of 274,505 fuel
charges, 266,313 raw steps, 188,435 expanded deliveries, and 81,945 meaning
lookups, plus 8,192 nodes, 78,020 artifact bytes, and 11,448 DVI bytes.

| Counter                       |             Baseline |                After |                Delta |
| ----------------------------- | -------------------: | -------------------: | -------------------: |
| Public `memcpy`               | 243,220 / 26,917,432 | 222,733 / 22,000,552 | -20,487 / -4,916,880 |
| Selected 240-byte caller bin  |   20,487 / 4,916,880 |                0 / 0 | -20,487 / -4,916,880 |
| Public `memmove`              |    8,201 / 1,376,552 |    8,201 / 1,376,552 |                0 / 0 |
| Allocations / requested bytes |  26,140 / 17,099,797 |  26,140 / 17,099,797 |                0 / 0 |

Thus the whole-process `memcpy` delta is exactly the removed caller bin.
`memmove`, allocation, requested bytes, semantic work, and output sizes are
unchanged, and no `execute_cold_episode` or callback-wrapper copy bin appears
in the after report. Both caller tables report zero overflow.

Ignored evidence is under `target/umber2-66p0.8.40.117/`. The baseline binary,
raw report, and symbolized report SHA-256 values are respectively
`181c26eb90f82c034ee7d1f7af75324684e1e4cb9f7c9a2d2ce49563eaf76fd3`,
`6e9a91b1421a340eebf3b9481c34489dc30c3abdb925dfe0f67aa59db568d390`,
and `98574259a66ad53dea760ac3c0bca8418ee9aa9961d552246dec613826ab29be`.
The after values are
`5ad2a2d893f5e7e9dc579f1b64ef9bb14a8dab62eecd2dfdc985fc29570938d7`,
`3e09182c5e1341ab8523a35d74c266c1440236db0f314106d418924657c09b84`,
and `eaa8d72fa02a9c6bf496d2641aea2f3a0a2109bad61e0ee8227a6fd600b5cf16`.
