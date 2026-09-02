# `umber2-66p0.8.40.163`: integrated profile blocked by stack-resident command state

## Attempted authority boundary

The intended sole accepted row was the established 50-million-command arXiv
authority on commit `3220d8c8d6580ea4873fb37204f3ddfc02a516e4` (tree
`2cec0e853925bdc7659d3f08cbadd2622f1f91a6`). The Rust 1.93.0 profiling
binary has SHA-256
`45c625938ffc7c7e722ca0c63880d608657e4ad3a4697ec798e8dfc40716d4c3`,
ELF build ID `2e27e2971f72dd9c0160ecb144e45102fa210946`, and size
422,882,832 bytes. The checked public-copy interposer has SHA-256
`8afd6ca34d91c28fccbccff4273a2f47dfa61861d3bceb53cf2c14dc7d2be16e`.

The inputs remained byte-identical to `.157`: `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
distribution-manifest SHA-256
`a68acebc1a83fd4ec0ce8c3baed4e8fe01de9b37e5a878b2f1fc203c2f20662f`,
distribution aHash64 `df66c327ae636145`, ordered 123-key closure SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`,
and source epoch `1787080434`. The command retained 50,000,000 canonical
fuel, 100,000,000 executor steps, 90 seconds, 1,536 MiB aggregate RSS, 199 Hz
`cycles:u`, 8,192-byte DWARF callchains, and simultaneous user
`cycles,instructions`. There was no CPU hold or 100M execution.

No accepted authority row exists. The release binary diverged after format
materialization and before reaching the 50M endpoint, allocation census, or
copy-probe destructor. Consequently there is no current semantic vector,
wall/user/system authority, cycles/instructions authority, sample/loss
authority, peak-RSS authority, allocation row, public-copy row, or grouped
self-CPU table to compare honestly with `.157`.

## Preserved failed-start evidence

The ordinary 8 MiB native stack aborted with Rust stack overflow. A diagnostic
64 MiB stack repeated the same failure. Neither reached the terminal fuel
publication; both left an empty copy file because abort bypassed interposer
reporting. Their launch-only counters are not performance results:

| Launch ceiling | Outcome       | Outer wall/user/system | Outer peak RSS |      Perf cycles/instructions | Samples |
| -------------- | ------------- | ---------------------: | -------------: | ----------------------------: | ------: |
| 8 MiB stack    | stack abort   |   5.13 / 1.24 / 0.75 s |     62,632 KiB |   934,363,495 / 1,916,549,943 |      69 |
| 64 MiB stack   | stack abort   |   4.61 / 1.49 / 0.90 s |    119,744 KiB | 1,782,774,385 / 2,193,730,225 |     118 |
| unlimited      | RSS guard 125 |   2.20 / 0.36 / 0.21 s |              — |                             — |       — |

The last launch was diagnostic only and is rejected as an authority: the
unchanged 1,536 MiB guard observed 1,660,004 KiB aggregate RSS and terminated
the process group. The guard reaped the reported survivors. The RSS guard was
not raised or removed, and unlimited-stack execution is not an acceptable
authority configuration.

## Exact stack-resident construction

An exact layout probe compiled under the same profiling feature resolution
measured these retained-generation components:

| Type                   | Exact `size_of` |
| ---------------------- | --------------: |
| `ResidentCommandState` |     5,112 bytes |
| `CommandVisibleState`  |     8,888 bytes |
| `Universe`             |    52,184 bytes |
| `CommandContext`       |        80 bytes |

`Universe::new` constructs `ResidentCommandState` by value inside
`CommandVisibleState`, also by value inside `Universe`
(`universe.rs:1518-1563`). Loaded-format construction then returns that
52,184-byte `Universe` by value through this path:

```text
RetainedStateGeneration::from_format_owned_with_page_node_identity_demand
  -> materialize_retained_format
    -> Universe::new_format_candidate
      -> Universe::new
  -> PhysicalStateGeneration { universe, ... }
  -> ReachabilityStore::insert_generation
```

The current profiling binary confirms the resulting stack cost rather than
merely reflecting the Rust source layout. The retained-format entry reserves
`0x27518`/161,048 local bytes, or 161,104 bytes including six saved registers
and its return address. `materialize_retained_format` reserves
`0x171f8`/94,712 local bytes, or 94,768 bytes complete. Its nested
`Universe::new` reserves `0x3ed8`/16,088 local bytes, or 16,144 bytes complete.
At the deepest constructor call these frames retain 272,016 bytes before any
older caller frame. The 5,112-byte all-purpose resident aggregate is therefore
not a harmless reference bundle: it participates in repeated by-value
construction and moves as part of the much larger `Universe` owner.

After materialization completes, `Universe::command_context` and
`Universe::with_command_context` (`universe.rs:3505-3530`) construct the
80-byte reference-only `CommandContext` from four capabilities: a mutable
reference to the resident aggregate, an admitted dense-state view, a checked
page-node arena, and a mutable page-builder reference. However, callers form
that view at individual command-operation boundaries rather than borrow it
once for the uninterrupted execution episode.

## Observed overflow path

Format materialization and reachable-state identity complete before the
captured overflow. Execution then repeats this live call cycle:

```text
expanded_delivery_entry
  -> expand_classified_occupied
  -> expand_the
  -> scan_internal_value_or_zero
  -> get_x_token_into
  -> expanded_delivery_entry
```

The exact captured retained-generation monomorphs reserve these x86-64 stack
components. Each also saves six registers (48 bytes) and owns an 8-byte return
address:

| Component                     | Local reservation | Complete frame growth |
| ----------------------------- | ----------------: | --------------------: |
| `expanded_delivery_entry`     |       `0x378`/888 |             944 bytes |
| `expand_classified_occupied`  |     `0xc88`/3,208 |           3,264 bytes |
| `expand_the`                  |       `0x288`/648 |             704 bytes |
| `scan_internal_value_or_zero` |       `0x228`/552 |             608 bytes |
| **One recursive cycle**       |             5,296 |       **5,520 bytes** |

The 5,520-byte live cycle explains exhaustion at both tested stack ceilings.
The only release/test control-flow difference found in the new replay path is
`SegmentedReplayLane::advance_sequential`'s `cursor.remaining == 0` return,
which is compiled only under `cfg(test)` before the direct segment read and
decrement. That is a concrete release-equivalence hole, but the preserved
samples do not contain token values and therefore do not prove it is the sole
cause of the recursive `\the` stream. It must be covered by the source fix,
not promoted to an authority conclusion from an aborted run.

The required source fix is tracked once as P0 `umber2-66p0.8.40.165`.
`Universe` must keep durable stores separated by semantic lifetime rather than
preserve the 5,112-byte by-value `ResidentCommandState` merely because command
operations access all of its fields. `CommandContext` must remain a small,
reference-only view, borrowed once per uninterrupted execution episode and
reused across its commands. The successor also owns release-equivalent replay
exhaustion coverage and must restore the exact 50M semantic endpoint on the
ordinary 8 MiB stack under the unchanged RSS guard. This issue makes no
production change and deliberately does not select a further performance
simplification from invalid execution.

## `.157` comparison and evidence

The compatible `.157` authority remains the last valid comparison row:
`(50000000,49903532,9457781,15936698,35326903,4203)`, 19.32 seconds wall,
19.54 user, 2.19 system, 40,323,285,089 cycles, 71,563,607,021 instructions,
3,371 samples with zero loss, and 166,820 KiB peak RSS. Its named allocation
total was 3,346,017 calls / 26,510,056,981 requested bytes; public copies were
9,203,569 `memcpy` calls / 1,245,500,251 bytes and 21,610 `memmove` calls /
2,827,518 bytes. The current tip supplies no valid delta against those values.

Ignored evidence is under `target/umber2-66p0.8.40.163/`. The SHA-256 values
for the 8 MiB, 64 MiB, and guard-terminated `perf.data` files are respectively
`62bc45d8574a083f92eed5a60286585dd70409a890956a1e13a181c4b67ccb9e`,
`f83116ac2d07d5c11ae3943416448cd713365c95cf742c15bb1fd47818015990`, and
`72b1e343170db67cd2c4dd3012eb5c976e6b4bc1da065dc13a660ae59b893076`.
Their stderr SHA-256 values are respectively
`19477414129ff5c75b1bac3a2b0e916d2efc56d8a3b070d1b135cb9543b45541`,
`6561b775dcdfc76ce726ba85ca6194bcb9a33d2c58c72598ff86f15630d03b96`,
and `41a6a1a369c2219bc02debebeb918e5de96dc279c863730a48a5250862a39e63`.
