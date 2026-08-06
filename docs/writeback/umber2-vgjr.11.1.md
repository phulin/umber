# umber2-vgjr.11.1 — direct canonical TFM runtime construction

Source snapshot: `6618b344135f7e930c72faecd192c1e20f067e41`.
Implementation commit: `991fc7fcfafbfe5d232456ccb76b238baff05384`.

`tex-fonts::FontMetrics` is now the sole retained classic metric authority.
The TFM parser keeps private raw character, lig/kern, kern, and extensible
records only until all structural, reference, chain, and error-precedence
checks finish, then directly constructs canonical metrics. `TfmFont` retains
those metrics with the header, selected size, padded parameters, and TeX82
`font_info_words`. Its consuming constructor creates `LoadedFont` for both
executor and VF paths, eliminating their duplicate assembly logic.

The removed predecessor is the public raw TFM object graph: character bounds,
raw character metric indices and tags, raw lig/kern words and actions, kern
indices, ligature deletion records, extensible recipes, and the later
`font_metrics` conversion. This intentionally contracts the public Rust API.
Repository inventory found only crate tests and the live `tftopl` reference
tool using those DTOs; frozen formats, output, wire schemas, and runtime callers
already use `FontMetrics`. The reference tool now compares canonical character,
boundary, lig/kern, next-larger, extensible, and parameter queries directly.
OpenType MATH (`umber2-vgjr.11.2`), realized font identity
(`umber2-vgjr.11.3`), and binary fixture subsetting are excluded from this
slice.

The implementation adds 183 and deletes 242 production Rust lines (net -59),
adds 73 and deletes 57 Rust test lines (net +16), adds 53 and deletes 34
live-reference tool lines (net +19), and adds/deletes four guidance lines.
Deletion categories are: raw public model and conversion boundary +54/-187;
private validation and direct projection +125/-30; runtime caller construction
+4/-25; proof migration +126/-91. Total authored change is 313 additions and
337 deletions, a 24-line net deletion. No fixtures, generated files, formats,
wire schemas, lockfiles, or binary assets changed. The 800--1,200 production
LOC forecast belongs to all three program-11 children; the two larger sibling
slices remain open, so this child does not establish a forecast shortfall.

Validation used finite timeouts and no within-slot overlap. All 80 focused
`tex-fonts` tests passed under `MemoryMax=512M` at a 79,532,032-byte cgroup
peak; all nine VF tests passed at 39,239,680 bytes; 99 selected format tests
passed with three ignored at 211,767,296 bytes; all 33 fixturegen tests passed
at 95,952,896 bytes; and the live `tftopl` comparison passed at 24,702,976
bytes. The isolated fuzz target built and completed 10,000 inputs at 25,923,584
bytes. Independent focused measurements were 40,460 KiB for 79 pre-final
font tests and 51,164 KiB for nine VF tests, showing no runtime growth.

The final complete routine suite passed under `MemoryMax=1G` at a
631,353,344-byte peak with no memory event. After correcting an invalid
host-toolchain attempt, all four `scripts/check.sh` gates passed with
`CARGO_BUILD_JOBS=6` under the same cap at a 105,771,008-byte peak with no
memory event. The invalid attempt used the transient service's old Cargo and
missing `dprint`; it is not evidence. The earlier full-suite source-audit
failure was an exact-coordinate ledger movement caused by the intentional
12-line executor contraction; only those two reviewed coordinates were
updated, with no new or broadened exception.
