# `umber2-66p0.30`: rejected canonical-resolution consolidation

## Decision

The resolver-owned accounting/decode consolidation was rebased onto
`a7e0a44496c6fd6a97d3ad4a6c3ef29c6e2f04a8` as candidate
`abe60694f`. It removed real duplicate work and simplified ownership, but
fresh clean whole-engine evidence did not meet the no-regression gate. The
candidate is therefore rejected and the production tree retains current
main's uniform resolver and stable borrowed `CommandContext` design.

The rejected candidate moved meaning-lookup fuel accounting into the exact
control-sequence/active-character resolution match, removed the second spelling
decode, derived ordinary observer identity from the resolved meaning, replaced
the retained identity plus outer-recovery boolean with one exceptional
adjustment discriminator, and deleted the permanently empty macro-observation
operand. It added no cache, special fast path, heap owner, or lifetime
machinery. Representative optimized resolver text shrank from `0x7f5` to
`0x5e5` bytes and `get_next_canonical` from `0x1f3f` to `0x1e51` bytes.

## Exact boundary

The comparison used the authenticated arXiv `2606.12566` workload:

- selected `ArXiv.tex` SHA-256
  `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
- schema-12 format object `ahash64-v1-2b924b5bba05d8a0`, SHA-256
  `ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
- packed distribution root `721e833071d92bba`, with manifest SHA-256
  `4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`;
  and
- the unchanged 123-key prefetched closure.

One force-frame-pointer ELF per revision served every accepted row. The main
ELF has SHA-256
`26b094d014afab0ff49f39727f3ece9a5b0c8dbf6f9d637abe2d037e256fe56e`
and build ID `cd796c559aaa90d6329d98c7299aa4c06e8538fb`; the candidate
ELF has SHA-256
`ca5438ea5561e2b8c14c7b4affc28ffba851507f63bfcb8ed72b4cddc30d0bb5`
and build ID `681e5f95bf9d503c8088c4eb1229a1109bb2cb1c`.

The accepted capture rows ran under one uninterrupted
`flock /tmp/umber-perf-host.lock`. Each row independently had CPU `some` and
`full` `avg10=0.00` both before and after, and its saved process censuses had
no Cargo, rustc, Umber, perf, or Ansible peer. Every control, capture, and
hardware-counter row reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions, with identical distribution telemetry.

## Whole-engine result

The clean 199 Hz `cycles:u` captures contained 1,377 main and 1,435 candidate
samples with zero lost events. Rows below are non-additive: self is the leaf
bucket and ancestry includes every sample containing the owner.

| Owner                                   |    Main cycles | Candidate cycles |  Change |
| --------------------------------------- | -------------: | ---------------: | ------: |
| Total weighted capture                  | 17,370,569,875 |   17,578,279,907 |  +1.20% |
| `CurrentCommand::resolve_into` self     |    921,571,786 |    1,130,230,549 | +22.64% |
| `CurrentCommand::resolve_into` ancestry |  1,024,472,486 |    1,334,357,063 | +30.25% |
| `get_next_canonical` self               |  1,721,802,829 |    1,663,810,232 |  -3.37% |
| `get_next_canonical` ancestry           |  4,420,095,428 |    4,938,629,702 | +11.73% |
| shared `memmove` self                   |    827,058,594 |      737,030,235 | -10.89% |
| shared `memmove` ancestry               |    861,600,795 |      775,951,152 |  -9.94% |

Frequency-sampled symbol buckets varied materially across otherwise matched
captures, so three additional clean alternating `perf stat` pairs measured the
whole process directly. Their candidate `cycles:u` deltas were `-1.94%`,
`+1.38%`, and `+1.16%`; the median values were
`17,223,092,982 -> 17,421,777,352` cycles (`+1.15%`). Median task clock moved
`6,775.80 -> 6,893.70` ms (`+1.74%`), wall time `6.96 -> 7.05` seconds
(`+1.29%`), user time `7.59 -> 7.67` seconds (`+1.05%`), system time
`0.78 -> 0.79` seconds (`+1.28%`), and peak RSS
`325,752 -> 325,792` KiB (`+0.01%`). Retired instructions consistently fell;
their medians moved `32,904,549,852 -> 32,312,118,414` (`-1.80%`).

Thus the earlier single capture's `+4.11%` magnitude was not stable, but its
whole-engine concern was not disproven: fewer instructions did not produce a
cycle or task-clock win. The architecture contract requires measured
whole-workload improvement, so local simplification and reduced copy ancestry
are insufficient promotion evidence.

## Verification and disposition

Before rejection, the rebased candidate passed 243 `tex-command` unit tests,
18 boundary integration tests, the full `cargo test -q --tests` suite, and all
four `scripts/check.sh` gates. The warmed
`destination_directed_warm_delivery` row completed 24,576 deliveries with zero
allocation calls and zero requested bytes. No semantic or allocation failure
caused the rejection; the measured whole-engine slowdown did.

The candidate implementation and its superseded acceptance note were reverted.
No follow-up issue is warranted without a different simplification that can
clear the same authenticated whole-engine gate.
