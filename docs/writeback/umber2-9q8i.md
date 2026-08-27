# `umber2-9q8i`: integrated stable-context and scalar-frame profile

## Authority and controls

The measured checkout is exactly
`3a91f1d2691b693c5eb2321504d8f175f4a61336`. Production source under
`crates/` and the workspace manifests has no diff from
`a7e0a44496c6fd6a97d3ad4a6c3ef29c6e2f04a8`: the intervening canonical
resolver candidate was rejected and reverted. One force-frame-pointer
executable served cold acquisition, warmed control, `cycles:u`, and public-copy
census rows. It is 387,008,440 bytes, has ELF build ID
`cd796c559aaa90d6329d98c7299aa4c06e8538fb`, and has SHA-256
`26b094d014afab0ff49f39727f3ece9a5b0c8dbf6f9d637abe2d037e256fe56e`.

The authenticated input tuple is unchanged from `b95e482b8`'s integrated-copy
authority:

- arXiv `2606.12566` source archive SHA-256
  `05a491fc231c85c5827f1dd1b41f80c361f300898d2b3830601c121b0e6d8a2a`;
- selected `ArXiv.tex` SHA-256
  `816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`;
- schema-12 format object `ahash64-v1-2b924b5bba05d8a0`, SHA-256
  `ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`;
- packed root `721e833071d92bba`, whose `manifest-v8.json` has SHA-256
  `4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`;
  and
- ordered 123-key closure SHA-256
  `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

The separate cold row acquired 124 distribution resources and materialized the
same 572-file, 48,775,587-byte private cache closure as the authority. No
warmed row emitted an acquisition. Every row reported the same distribution
telemetry: 164 reads and validations over 46,856,310 bytes, 163 shard loads and
packed validations over 46,848,325 bytes, and 225 selections for 245 keys and
65,121,544 bytes.

One outer `flock /tmp/umber-perf-host.lock` remained held across the complete
warmed control, perf, and census sequence. CPU `some` and `full` pressure had
`avg10=0.00` at both window boundaries and at every row boundary. Saved process
censuses contained no Cargo, rustc, Umber, or perf peer. Load moved from 1.73
to 2.25 on 24 online CPUs without CPU pressure. An earlier launch was rejected
before any comparable row started because CPU `some avg10` was 0.31; it
contributes no evidence.

## Exact endpoint and process measurements

Every accepted row intentionally returned status 1 at the exact fuel limit and
reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)`: fuel charges, token-frame
steps, expanded deliveries, meaning lookups, scanner tokens, and deferred-write
expansions. Standard output was empty and no partial PDF was published.

| Row                            | Wall (s) | User (s) | System (s) | Peak RSS (KiB) |
| ------------------------------ | -------: | -------: | ---------: | -------------: |
| `8c45ae8fb` warmed control     |     7.76 |     8.50 |       0.87 |        325,788 |
| `3a91f1d26` warmed control     |     7.63 |     8.30 |       0.87 |        325,832 |
| `8c45ae8fb` `cycles:u` perf    |     8.19 |     8.78 |       1.11 |        325,916 |
| `3a91f1d26` `cycles:u` perf    |     8.25 |     8.67 |       1.11 |        325,780 |
| `3a91f1d26` public-copy census |     9.31 |    10.20 |       1.11 |        365,548 |

The single warmed control is 0.13 seconds lower in wall time (1.68%) and 0.20
seconds lower in user time (2.35%), with unchanged system time and 44 KiB
higher peak RSS (0.01%). The perf and interposer rows have different observer
overhead and are attribution evidence, not latency controls.

## Zero-loss absolute cycles

The fresh 199 Hz `cycles:u` frame-pointer capture contains 1,508 samples, zero
lost samples, and 17,761,407,065 weighted cycles. This is 188,907,445 cycles
(1.05%) below the 1,528-sample, 17,950,314,510-cycle authority. The fresh
`perf.data` SHA-256 is
`dbbfa840e666bc8f74e81765d2868711d1afa56c834a45f4248a184f0e87cf6e`.

The rows below deliberately report both self and complete ancestry. They are
not additive: scanning contains delivery and resolution, while operation
preparation contains scanners. The recommendations select disjoint concrete
representations rather than treating overlapping parents and children as
separate work.

| Owner                                   |     Base self | Base ancestry |    Fresh self | Fresh ancestry | Ancestry change |
| --------------------------------------- | ------------: | ------------: | ------------: | -------------: | --------------: |
| `scan_toks_buffers`                     |    24,703,628 | 5,441,437,559 |    24,272,845 |  5,765,100,374 |    +323,662,815 |
| `get_next_canonical`                    | 1,692,834,944 | 5,074,271,840 | 1,705,913,210 |  4,841,835,945 |    -232,435,895 |
| `MainControl::prepare_operation`        |   193,261,759 | 2,250,044,709 |   183,192,510 |  2,326,694,461 |     +76,649,752 |
| `CurrentCommand::resolve_into`          | 1,273,108,005 | 1,467,056,711 | 1,278,016,510 |  1,456,455,751 |     -10,600,960 |
| shared libc copy kernel                 | 1,260,898,789 | 1,307,634,119 | 1,114,074,417 |  1,140,336,835 |    -167,297,284 |
| `DistributionResolver` batch resolution |             0 |   748,149,894 |             0 |    699,559,315 |     -48,590,579 |
| `ValidatedPackedShard::new`             |             0 |   583,434,111 |             0 |    542,117,949 |     -41,316,162 |

The completed destination-directed command, scalar-frame, borrowed-context,
argument-fact/span, and packed-shard owners are not reopened. Canonical
resolution remains substantial, but the measured candidate for that owner was
rejected on whole-engine cycles and is also excluded.

## Public copy traffic

Libc resolves public `memcpy` and `memmove` to the shared
`__memmove_avx_unaligned_erms_rtm` implementation, so the API census and shared
kernel cycles are parallel evidence rather than an invented per-API cycle
split. Both census tables had zero caller and size overflow.

| Public API | `8c45ae8fb` calls | Fresh calls | Call change | `8c45ae8fb` bytes |   Fresh bytes |    Byte change |
| ---------- | ----------------: | ----------: | ----------: | ----------------: | ------------: | -------------: |
| `memcpy`   |        36,542,915 |  33,616,263 |  -2,926,652 |     6,120,261,227 | 4,985,212,288 | -1,135,048,939 |
| `memmove`  |            52,070 |      52,070 |           0 |         4,795,428 |     4,795,428 |              0 |

Public `memcpy` fell 8.01% in calls and 18.55% in bytes. The accepted large
scalar result rows and the four redundant borrowed-context facade rows are
absent; only 768 cold direct `scan_integer` calls totaling 608,256 bytes remain
from the former 712--792-byte scalar family.

Two recommendation-owned copy families remain exact in the fresh census.
`register_incremental_inputs` performs 21,250 calls and copies 465,275,787
source bytes; `refresh_candidate_files` has 271,082,555 full-ancestry cycles,
including 177,514,232 cycles as the shared-kernel immediate parent.
`ListJournal::record_once` performs 233,236 624-byte calls and moves
145,539,264 bytes. Pending horizontal-run projection/value rows are distinct
and are not included in that journal count.

## Exactly three next CPU targets

1. `umber2-66p0.32`, _Collapse input-stack bookkeeping into owned frame
   transitions_. Three disjoint self leaves total 569,303,558 cycles (3.21%):
   per-push `Arc<Mutex>` maximum accounting, open-depth identity scans/clones,
   and retirement. Put scalar maxima on the existing singular session owner
   and move the top frame's already-owned nesting record through retirement,
   deleting synchronization, scans, and clones without a cache, special path,
   heap indirection, second input representation, or new lifetime owner.
2. `umber2-7asg.8`, _Eliminate incremental input refresh ownership copying_.
   Transfer immutable workspace bytes once and remove the repeated COW/BTree
   ownership refresh behind the 271,082,555-cycle ancestry and 465,275,787-byte
   copy row. This consolidates workspace/session ownership; it must not add a
   cache, fast path, heap indirection, or generation-long lifetime.
3. `umber2-7asg.13`, _Reduce mode inverse-journal entry memcpy_. Replace the
   maximum-sized 624-byte enum transfer with narrow tagged inverse payloads,
   deleting 145,539,264 bytes of ordinary executor transfers without whole-list
   snapshots, boxing, caches, compaction, special paths, or retained lifetime
   machinery.

All three Beads are linked as discovered from this profile. Raw profiles,
process censuses, identity receipts, exact copy tables, and analysis output
remain issue-private under `target/umber2-9q8i/`; no production source, input,
format, distribution, cache policy, or semantic behavior changed.
