# `umber2-7nnp`: post-delivery exact CPU profile

## Measured authority

The coordinator satisfied the issue's “integrate the first freed slot” clause
before dispatch by fast-forwarding the completed destination-directed caller
migration. The measured tip is therefore exactly
`e9f90b57b21043822d8917c9857c7f2776b3082a`; no additional optimization was
pending or integrated in this profiling branch.

The issue-private force-frame-pointer executable was built once with Rust
1.93.0 and copied out of Cargo's build directory before any row ran. It is
386,333,136 bytes, has ELF build id
`d8ff87a9383a157f28bdf532a0975ae2555e0201`, and has SHA-256
`9b1d212ae9df161332f7ae5cc8dcfe05876be41955e9542a14a7f7ed3286c04f`.
Control and perf executed this same path, device, and inode.

The authenticated workload identities are:

- arXiv `2606.12566` source archive: 46,654,107 bytes, SHA-256
  `05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`;
- selected `ArXiv.tex`: 116,384 bytes, SHA-256
  `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
- schema-12 `pdflatex.fmt`, object
  `ahash64-v1-2b924b5bba05d8a0`: 1,139,703 bytes, SHA-256
  `ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
- packed distribution root `721e833071d92bba`, whose 7,985-byte
  `manifest-v8.json` has SHA-256
  `4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`;
  and
- ordered 123-key offline closure, SHA-256
  `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

The fixed clock, 45-second wall guard, 1,536 MiB aggregate-RSS guard,
offline policy, locale, source tree, and expansion-fuel limit were identical
between rows. The final authority began with a 572-file warmed issue-private
cache whose ordered content digest is
`7f44986f7f82db8b4665cb614dd9aa9d3d66fa23b689381d85be19822f67c672`.

## Quiet-host control and profile

The authoritative control and perf processes ran consecutively while one
outer `flock /tmp/umber-perf-host.lock` remained held. Immediately before and
after the pair, CPU `some` pressure had `avg10=0.00`, CPU `full` pressure was
zero, and there was no Cargo, rustc, Umber, or perf peer. Host load changed
from 2.82 to 2.64 on 24 online CPUs. Process snapshots are retained beside
each row.

Both processes intentionally returned status 1 at exact fuel exhaustion and
preserved the vector
`(20000000,19913119,2218327,6020965,16785710,4011)`: fuel charges, token-frame
steps, expanded deliveries, meaning lookups, scanner tokens, and deferred-write
expansions.

| Row                    | Wall (s) | User (s) | System (s) | Peak RSS (KiB) |
| ---------------------- | -------: | -------: | ---------: | -------------: |
| Warmed control         |     8.40 |     8.91 |       0.96 |        326,388 |
| Warmed `cycles:u` perf |     8.24 |     8.57 |       1.06 |        326,768 |

The perf row is attribution evidence, not a latency improvement claim. Its
sampling and observer overhead differs from the control, and one pair is not
a timing distribution.

An earlier empty-cache diagnostic acquired 124 distribution resources and
grew the private cache from zero to 572 files. The later warmed control and
authoritative perf row emitted no acquisition record. This separates cold
distribution materialization from the hot expansion work. The first
diagnostic capture's timing and CPU profile are rejected because its saved
process snapshot exposed a concurrent slot-3 profiling build; none of its CPU
numbers is used below.

## Zero-loss full ancestry

The authoritative `perf record -F 199 -e cycles:u --call-graph fp` capture
contains 1,482 samples, zero lost samples, and exactly 18,671,810,762 weighted
cycles. `perf.data` has SHA-256
`1aa47a4d81b504b51e9a847177daef4f9b1a253579569afab6d7b79e0df8aeaf`.
The complete 46,307-line callchain expansion has SHA-256
`054657639f437d18dccde838473c2996492800e783ee69267278aa4cbaff5836`.
Raw events, flat self periods, complete caller/callee reports, inclusive
ancestry totals, and immediate-parent attribution live under
`target/umber2-7nnp/perf-quiet-20m/`.

The ranked owner evidence is deliberately non-additive: an inclusive scanner
sample can also contain canonical delivery, resolution, and executor frames.

| Rank | Full-ancestry owner or disjoint self owner              | Weighted cycles | Complete share | Interpretation                                                                                       |
| ---: | ------------------------------------------------------- | --------------: | -------------: | ---------------------------------------------------------------------------------------------------- |
|    1 | `scan_toks_buffers` inclusive                           |   5,426,599,327 |        29.063% | Macro-definition scanning is the widest hot command subtree; `scan_toks_inner` is 25.284% inside it. |
|    2 | `get_next_canonical` inclusive / self                   |   5,184,322,841 |        27.766% | It remains the dominant reusable delivery kernel; its disjoint self cost is 1,702,861,638 (9.12%).   |
|    3 | `MainControl::prepare_operation` inclusive              |   2,868,925,949 |        15.365% | This overlaps scanner/delivery ancestry; its self cost is only 156,825,740 (0.84%).                  |
|    4 | shared libc `memmove` ancestry                          |   1,459,204,867 |         7.815% | Heterogeneous immediate parents require the existing copy-owner issues, not one aggregate rewrite.   |
|    5 | `CurrentCommand::resolve_into` inclusive / self         |   1,263,275,820 |         6.766% | Every sampled resolver call is below canonical delivery; self cost is 1,160,526,973 (6.22%).         |
|    6 | hot `DistributionResolver::resolve_batch_with_prefetch` |   1,124,992,990 |         6.025% | This remains after cache warming; `ValidatedPackedShard::new` accounts for 948,766,316 (5.081%).     |

The copy-kernel ancestry is concrete rather than assigned wholesale to the
first caller. Its largest immediate sampled parents are
`MainControl::prepare_operation` at 200,487,468 cycles,
`VirtualCompileSession::refresh_candidate_files` at 186,694,031,
`execute_direct_episode` at 183,439,687, `scan_toks_buffers` at 89,249,864,
and `BTreeMap::clone_subtree` at 86,378,878. This refreshes existing
`umber2-7asg.1` and `umber2-7asg.10`; their inclusive values overlap and must
not be summed.

The packed-distribution result is not evidence against aHash64. In the same
hot profile, `manifest::validate_path` owns 145,706,724 self cycles (0.78%)
and `AHash64Hasher::write` owns 53,869,832 (0.29%), while packed-shard
validation has 948,766,316 inclusive cycles. The follow-up must first
attribute repeated validation rather than reverting authenticated identities.

## Integrated invariants and follow-up

Git ancestry and current source verify that the measured tip contains compact
control-sequence identities (`66c6a016b`), explicit checkpoint root forks
(`579711649`), packed primitive handles (`f27d2c0d7`), the aHash64 resource and
catalog migration (`d294aa194`, `ff3aafbf1`, and `ffbdb9861`), and the final
destination-directed caller gates (`e9f90b57b`). Focused tests cover the
primitive handle, checkpoint fork, compact command delivery, and aHash64
contracts; the documentation formatting gate is recorded at issue close.

Fresh CPU-first work is filed as `umber2-66p0.30` for the post-delivery
canonical command kernel and `umber2-66p0.31` for hot packed-distribution
validation. Both are children of the active runtime-performance epic and were
discovered from this issue. Existing `umber2-7asg.10` retains the overlapping
macro-scan and copy seam, while `umber2-7asg.1` retains prepared-operation
transfers. No production source, representation, fixture, distribution,
format, or cache policy changed in this issue.
