# umber2-awgc.5.3.3: Fused Hot-Path Acceptance

## Outcome

The expansion and ranked unexpandable cuts are integrated at `8d765fd69`.
`MainControl` owns one session-lived `PersistentCommandInterpreter`, and that
interpreter owns the only `CommandState`. Borrowed `CommandProcessor` facades
lend this state; they do not create another command machine or executor.

The ranked expansion path keeps the live command borrowed until a typed
resource barrier. The ranked definition, `\let`, catcode, and ordinary-group
path scans to a family-sized `HotOperation` and applies it through the same
interpreter transaction. It does not construct `ScannedStep`,
`PreparedOperation`, or an apply clone. The compatibility path remains the
only owner of those universal values.

## Exact structural census

The final profiling binary was built from the integrated source and has
SHA-256
`c9de6aee3b03fcef7197b023cdfc1dcda90aeb2ce11784e0851de09eb8aaf988`.
Both rows use `SOURCE_DATE_EPOCH=1787080434`, `LC_ALL=C.UTF-8`, the restored
schema-11 format, the schema-3 distribution with manifest SHA-256
`560ab65f2a4933879b05e47554a9d94434ec1e94ff8f6caa163d26cde7fe35bd`,
the exact ordered 105-key offline closure, and source SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`.

| Counter                         | Frozen 6M | Final 6M | Frozen 12M | Final 12M |
| ------------------------------- | --------: | -------: | ---------: | --------: |
| Interpreter operation entries   |   190,107 |   78,140 |    392,501 |   158,474 |
| `ExpansionCommand`              |   445,714 |   10,497 |    930,475 |    23,055 |
| `ScannedStep`                   |    62,739 |   10,354 |    129,816 |    19,677 |
| `PreparedOperation`             |    62,735 |   10,350 |    129,812 |    19,673 |
| Ranked scan/apply DTOs removed  |         0 |   52,385 |          0 |   110,139 |
| Command-state or snapshot clone |         0 |        0 |          0 |         0 |

Processor entries fall 58.9% at 6M and 59.6% at 12M. Expansion materialization
falls 97.65% and 97.52%. Every remaining `ExpansionCommand` is in the exact
unranked complement recorded by
[`umber2-awgc.5.3.5`](umber2-awgc.5.3.5.md): `Meaning`, `Input`, `EndInput`,
`JobName`, `IfTrue`, `IfCat`, `IfDim`, `IfOdd`, `IfCase`, `IfVMode`, `IfHMode`,
`IfMMode`, `IfVoid`, `IfVBox`, `IfEof`, `Or`, `Unexpanded`, `Unless`,
`Scantokens`, `IfDefined`, `FileSize`, `StringCompare`, `ShellEscape`, and
`CreationDate`. Their exact sums are 10,497 and 23,055.

The unexpandable hot set is exact by construction: `Def`, `Edef`, `Gdef`,
`Xdef`, `Let`, `FutureLet`, `CatCode`, `BeginGroup`, the matching `EndGroup`,
and ordinary brace-group entry and exit, including the `Global` and `Long`
prefix state folded into the substantive operation. `direct_hot_candidate`
selects this set before compatibility scanning, and the compatibility scanner
marks each of its matching arms unreachable. The 52,385 and 110,139 removed
DTOs therefore contain the complete accepted hot set. The remaining 10,350
and 19,673 prepared operations are precisely its contextual complement:
unranked primitives, character and mode work, mismatched group recovery,
alignment, resource/effect/output barriers, and terminal handling. No member
of the admitted hot set can enter that complement.

## Work, allocation, and performance

The fixed-clock primary work vectors remain exact:

- 6M: `(6000000,5999815,507410,1718333,5352087,588)`;
- 12M: `(12000000,11999815,1177349,3506292,10599869,1182)`.

The warmed packed source, backup/replay, stored replay, and macro
matching/expansion rows each report zero allocations, requested bytes,
`Arc` retains, `Weak` retains or upgrades, weak-index work, and content hashes.
The 10,000-cycle warmed HotCore mark gate reports a 192-byte snapshot and zero
allocations, requested bytes, and retained bytes. These are hot-boundary
claims; the pinned process still includes genuine cold and all-live storage.

Guarded wall/RSS results are 7.22 seconds and 323,448 KiB at 6M, and 14.13
seconds and 453,156 KiB at 12M. Against the frozen child-1 rows of 18.69 and
47.19 seconds, this is a 2.59x and 3.34x phase speedup. The phase's 2x target
is met. The epic's separate 150-MiB final RSS target is not claimed here.

## Semantic and quality gates

The identical integrated tree has the following accepted receipts:

- canonical ignored TeX82 TRIP and e-TeX 2.6 e-TRIP both pass, including
  command, geometry, transcript, and normalized DVI channels;
- the exhaustive `tex-command-stream` run is `CLEAN` through every registered
  fixture with zero semantic divergences and zero advisory geometry
  differences;
- `cargo test -q --tests -- --test-threads=1` passes under the guarded
  single-job policy; and
- `scripts/check.sh` reports all four gates passed.

The combined acceptance audit reran the focused one-borrow and profiling
census controls on the primary checkout under a 1,536-MiB aggregate-RSS guard.
Both pass. The remaining full-tree and exact-row receipts are reused because
the source tree, commit, profiling binary, and immutable workload identities
are identical; no later source change exists to invalidate them.
