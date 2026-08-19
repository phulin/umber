# umber2-awgc.5.4: Typed Cold-Handler Cutover

## Outcome

The fused interpreter cutover is complete. `MainControl` owns one
session-lived `PersistentInterpreter` and the sole `CommandState`; hot and cold
execution borrow that same state. Ranked expansion, definition, `\let`,
catcode, prefix, and ordinary-group families remain in `hot_apply.rs` and do
not construct a cold operation. Uncommon, structurally large, resource,
effect, output, diagnostic, alignment, math, and PDF commands cross the typed
`ColdOperation` barrier.

The old universal runtime names `ScannedStep` and `PreparedOperation` no longer
exist. Their surviving profiling counter labels are intentionally stable so
the fixed-clock rows remain comparable with the frozen child-1 baseline. There
is no compatibility executor or second source/expansion machine: the cold
scanner receives a borrowed `CommandProcessor`, returns an operand-bearing
`ColdOperation`, and the cold apply modules mutate the same `MainControl`,
`Universe`, `ModeNest`, and node-list owner. Retry preserves that typed value
only across a real resource boundary. Diagnostics, provenance, observations,
effects, and format behavior remain on the canonical path.

## Module and ownership audit

The former 19,683-line `main_control.rs` is 9,076 lines. The extracted cold
boundary is divided by responsibility:

| Module              | Lines | Ownership                                                   |
| ------------------- | ----: | ----------------------------------------------------------- |
| `cold/operation.rs` |   841 | Runtime-only typed operands; owns no interpreter or state.  |
| `cold/scan.rs`      | 2,654 | Uncommon operand scanning against the borrowed interpreter. |
| `cold/apply.rs`     | 3,365 | Cold stomach mutations against canonical execution state.   |
| `cold/alignment.rs` |   725 | Alignment continuations and typed alignment transitions.    |
| `cold/pdf.rs`       | 1,504 | PDF requests, effects, and observation publication.         |
| `cold/support.rs`   | 1,644 | Shared cold recovery and semantic helpers.                  |
| `cold/mod.rs`       |    20 | Boundary exports only.                                      |
| `hot_apply.rs`      |   391 | Family-sized fused hot operands and direct apply.           |

`fused_hot_and_typed_cold_dispatch_share_one_interpreter` is the structural
guard: it proves one `command_processor` constructor, one
`CommandProcessor::borrowed` boundary, and no `ColdOperation` or
`PreparedColdOperation` construction in `hot_apply.rs`. The directory and
module ownership contracts are recorded in `crates/tex-exec/AGENTS.md`,
`docs/main_control_replacement.md`, and `docs/tex_command_core.md`.

## Fixed-clock pinned acceptance

The profiling binary used for both rows has SHA-256
`2025f55505eee1d6d8d7a610a951a6143f92ed6fe87b5dd8c127b3a0c13e94c9`.
Both guarded runs use `SOURCE_DATE_EPOCH=1787080434`, `LC_ALL=C.UTF-8`, the
105-key offline closure, source SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
format SHA-256
`32ae8a46f86ecc3520b48ff6739fa413170f7b34c2263560d7d589abe1466a7b`,
and distribution-manifest SHA-256
`560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`.
The complete argv receipts are retained under
`target/umber2-awgc.5.4/pinned-{6m,12m}/command.json`.

| Fuel | Exact work vector                                   | Wall time | User time | System time | Maximum RSS | Child-1 wall | Speedup |
| ---: | --------------------------------------------------- | --------: | --------: | ----------: | ----------: | -----------: | ------: |
|   6M | `(6000000,5999815,507410,1718333,5352087,588)`      |    7.89 s |    8.40 s |      0.89 s | 323,164 KiB |      18.69 s |   2.37x |
|  12M | `(12000000,11999815,1177349,3506292,10599869,1182)` |   14.57 s |   16.17 s |      1.48 s | 452,776 KiB |      47.19 s |   3.24x |

Both rows end at the intended exact fuel boundary with exit status 1, empty
standard output SHA-256
`e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855`,
and the frozen work vectors. The phase's 2x child-1 target is met at both
clocks. The epic's separate 150-MiB final RSS target is not met or claimed.

The 12M structural census reports 158,474 interpreter operation entries,
23,055 `ExpansionCommand` materializations, 19,677 historical `ScannedStep`
counter events, and 19,673 historical `PreparedOperation` counter events.
Command-state clones, step-snapshot clones, and interpreter-borrow allocations
are zero. The residual apply-clone scope is eight calls and 1,060 requested
bytes, confined to cold retry/effect handling rather than the ranked path.

## Residual allocation and retained-memory census

The 12M row identifies the remaining large owners:

| Owner                      |     Calls | Requested bytes |
| -------------------------- | --------: | --------------: |
| Delivery and cold scanning | 1,553,960 |     301,866,458 |
| Semantic apply             |   696,252 |     199,387,101 |
| Weak-value store           |   528,512 |      43,664,824 |
| Evidence publication       |       907 |         684,668 |
| Interpreter construction   |         4 |             144 |
| Cold apply clone           |         8 |           1,060 |

Retained-ownership counters are 10,025,437 provenance atom retains,
3,212,090 provenance-list resolutions, 12,161,765 list-resolution comparisons,
2,130,732 `Arc` retains, 824,101 weak upgrades, 217,159 weak-index calls, and
101,704 content hashes. These are the next epic-scale memory targets, not
evidence of a second interpreter.

They are tracked separately as:

- `umber2-awgc.6.1`: move unobserved provenance retention to compact cold
  sidecars;
- `umber2-awgc.6.2`: eliminate ordinary weak-value and exact-index ownership
  work; and
- `umber2-awgc.7.1`: attribute and arena-pack the residual 453-MiB hot-core
  footprint.

## Semantic and quality gates

The integrated tree passes:

- `cargo test -q --tests -p tex-exec` in both default and `profiling` feature
  resolutions;
- the exact ignored canonical TeX82 TRIP and e-TeX 2.6 e-TRIP gates;
- the exhaustive `tex-command-stream` comparison, with `VERDICT: CLEAN`, zero
  gating divergences, and zero advisory geometry differences;
- guarded serialized `cargo test -q --tests -j1` for the full host workspace;
  and
- `scripts/check.sh`, whose final verdict reports all four gates passed.
