# `umber2-7asg.5.2`: Current-command resolution attribution

## Authority and boundary

The authority is the clean `e1f21f7275da208ddfc6b308027f9ddaa4e981a6`
profiling binary and zero-loss exact arXiv 20M capture under
`/home/phulin/umber/.worktrees/slot-2/target/umber2-7asg.4/`. The binary,
`perf.data`, raw event stream, and self-period report have SHA-256 values
`9ed990aea7d86083c2d03f30d54dfc00b0cbad27f272bc664fe8318e66af50a7`,
`9c0c95319c6f3413ab64fa0876718c1cf538e9beecfc9dbb16bdd785dee119d2`,
`6099d491e80cee9a2944a2b4951ad703089777a955e00615c5f19b66be80fe84`,
and `9fbeae802436a80bc29081b906b844eb7717398996ba315a192c31fd76353b96`.
It preserved the exact command-work vector
`(20000000,19913119,2218327,6020965,16785710,4011)` and captured 1,610
samples with no loss.

`CurrentCommand::resolve_into` owns exactly the work from decoding the already
delivered packed spelling through writing the final command value. Its caller
has already selected and advanced input, assigned the delivery stamp, and
recorded whether a mutable meaning read is required. The caller later records
the token frame, checks outer validity, adjusts alignment, and publishes
observations. Those operations and the caller's independent 2,215,870,854
self cycles belong to `umber2-7asg.5.1`, not this attribution.

The raw event stream assigns 72 samples and exactly 886,122,219 weighted self
cycles to `resolve_into`, or 44.50 weighted cycles per 19,913,119 completed raw
token frames. Twelve descendant samples bring the exact call-chain period sum
to 1,031,349,285 weighted inclusive cycles, or 51.79 per frame. The previously
reported 5.37% inclusive figure is rounded; the raw period sum is the absolute
authority.

## Exact frequency census

A temporary slot-3 investigative build added relaxed counters only at the
resolution boundary. Its serialized host run used the same source,
distribution, format, prefetch closure, offline policy, fixed clock, 20M fuel,
and guards. The binary has SHA-256
`c3bab9b0f99eafda1eecb398995b3bbdfdc1358befe1d69f5ae87408c0b41e44`.
Its stderr receipt under `target/umber2-7asg.5.2/census-20m/` has SHA-256
`803501c892444b1af4e0e71d2c5b6d4a973bdc9e72fe34d41de47520038f5894`
and reproduced the authority's complete command-work vector exactly. The
instrumentation was then removed; it is not a production change.

| Resolution input or result                     | Exact count | Share of all resolutions |
| ---------------------------------------------- | ----------: | -----------------------: |
| ordinary control-sequence token                |   5,978,399 |                   30.02% |
| defined active-character token                 |      34,195 |                    0.17% |
| undefined active-character token               |       8,372 |                    0.04% |
| ordinary character token                       |  13,890,139 |                   69.75% |
| frozen token                                   |       2,014 |                    0.01% |
| stray parameter token                          |           0 |                    0.00% |
| control-sequence or active meaning lookup      |   6,020,966 |                   30.24% |
| resolved macro owner transferred               |   2,610,646 |                   13.11% |
| non-ordinary current-command identity selected |     616,783 |                    3.10% |

The lookup count differs by one from the work vector's 6,020,965 because the
fuel failure occurs after the final resolution and before the caller records
that final meaning-lookup counter. The token-frame counter is recorded after
resolution and therefore matches the census exactly. This is counter ordering,
not changed command work.

## Concrete cycle attribution

The groups below partition every self sample by the annotated instruction at
which its period landed. Sampling skid limits instruction-level causality, so
adjacent loads, branches, and stores are grouped as one source operation. The
absolute group sums are exact period sums; they are not percentage products.

| Resolution-local operation                                                   | Weighted self cycles | Share of self | Cycles per frame |
| ---------------------------------------------------------------------------- | -------------------: | ------------: | ---------------: |
| unpack `TracedTokenWord`, validate character, and select token arm           |          197,804,175 |        22.32% |             9.93 |
| bounds-check and direct-index the 32-byte dense meaning-bank row by `Symbol` |          123,146,250 |        13.90% |             6.18 |
| branch on `MeaningWord` and construct/decode `ResolvedMeaning`               |          122,469,742 |        13.82% |             6.15 |
| clone, transfer, and balance generation-owned macro meaning state            |          134,139,011 |        15.14% |             6.74 |
| classify the exceptional `convert`/`xray`/expansion command identity         |           24,705,056 |         2.79% |             1.24 |
| copy the 48-byte optional source-provenance argument into the final command  |           61,682,672 |         6.96% |             3.10 |
| write remaining fields into the caller's 144-byte `Option<CurrentCommand>`   |          234,538,727 |        26.47% |            11.78 |
| **Total**                                                                    |      **886,122,219** |   **100.00%** |        **44.50** |

The inclusive descendants add 120,453,906 cycles in packed
`Meaning::decode_stored`, 12,370,165 in primitive-operand decoding, and
12,402,995 in the active-character interner lookup. These sum to the exact
145,227,066-cycle difference between self and inclusive attribution. Active
lookup applies to only 42,567 frames (0.21%), so it is not the default-path
representation target.

The archived DWARF and generated assembly establish the relevant layouts:

| Value                                               |  Size |
| --------------------------------------------------- | ----: |
| `TracedTokenWord`                                   |   8 B |
| `DefinitionId<G>`                                   |   8 B |
| `MeaningWord<G>`                                    |  24 B |
| `ResolvedMeaning<G>`                                |  24 B |
| meaning `BankCell<MeaningWord<G>>` stride           |  32 B |
| `Option<SourceProvenance>`                          |  48 B |
| `DeliveryStamp`                                     |  24 B |
| `CommandIdentity`                                   |   2 B |
| `CurrentCommand<G>` and `Option<CurrentCommand<G>>` | 144 B |

No allocator appears beneath `resolve_into`. Static meanings are scalar
copies and packed decoding. A macro meaning uses a generation-branded
non-atomic `ThinRc`; the current `DenseBank::get` first clones the complete
`BankCell`, `MeaningWord::resolve` clones the definition owner again, and
dropping the temporary cell balances one clone. The final command retains one
net owner, which is semantically required.

The final assignment already writes the 144-byte command directly into the
caller's `Option` slot. The assembly contains no command-sized `memcpy` or
`memmove`, heap allocation, name resolution, admission, cache probe, or
second command materialization. Provenance is not consulted by meaning or
dispatch; its 48-byte optional value is copied once because it is part of the
final command required by later diagnostics and observations.

## Clean-sheet alternatives

### Borrow the delivered meaning across command use

Rejected. The meaning cell is mutable TeX state. Assignment after delivery
must not alter the already delivered `cur_cmd`/`cur_chr`, and command use may
itself mutate state. Replay, outer recovery, executor retry, and typed resource
suspension can also retain the command beyond the lookup borrow. A borrowed
macro definition would not own its payload across assignment, group rollback,
or generation retirement. Extending the state borrow would therefore violate
both TeX semantics and the existing exclusive command episode.

### Retain the symbol and resolve lazily at dispatch

Rejected. This is a second inferred meaning cache in reverse: the spelling
would be reinterpreted after mutable assignments, while character and frozen
tokens would need parallel special cases. It duplicates the resolution rule,
breaks once-at-delivery semantics, and makes rollback and suspension depend on
the later state rather than the delivered command.

### Store a shared heap-indirected meaning row

Rejected. Per-cell or per-command `Rc`/`Arc` indirection would add allocation
and pointer chasing to all meanings, complicate assignment levels and journal
rollback, and make generation liveness depend on another ownership graph. It
does not remove the required final owner for a macro definition.

### Keep the current owned command but borrow the table row during resolution

Recommended for the implementation follow-up. The default control-sequence
representation already supplies the required stable `Symbol` index. A
borrow-scoped dense-bank accessor can bounds-check once, borrow the row, copy
or decode a static word, and clone a macro `DefinitionId` exactly once into
the final owned command. The borrow ends before any mutable command work. The
assignment level remains in the bank and does not enter `CurrentCommand`;
journaling, local/global restoration, operation rollback, and format loading
remain unchanged. The one final macro owner preserves generation, retry,
replay, and suspension lifetime exactly.

This change adds no cache, classifier, token fast path, semantic table,
allocation, heap indirection, or durable coordinate. It targets the redundant
32-byte `BankCell` copy and one retain/release pair on 2,610,646 macro
resolutions while leaving the already-direct final write and required
provenance intact. Implementation and before/after measurement are tracked by
`umber2-7asg.5.3`; this investigation makes no production change.
