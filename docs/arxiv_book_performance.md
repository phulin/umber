# Long-book PDF performance audit

Physical compaction of node and annex storage substantially reduces the book's
memory footprint. At the same 200-million-action endpoint, a quiet shipping
comparison of `99f75a4d9` and `7b9f40dbd` reduced median peak RSS by about
225 MiB, while median user CPU improved by only 2.0%. The fixed-fuel profiling
comparison below explains the storage reduction and the growth it does not
attribute. These measurements do not establish full-book completion.

The book still does not complete within the original guards. The latest
unchanged-guard run, `bf4a6a7ba`, reached the 500-million-action fuel limit after
118.12 seconds, before the wall timeout. Its receipt is
`target/perf-tex-copy-plan/ownership-annex-original-book/summary.json`.
This is a failed acceptance run, not a completion or a matched speedup claim.
Keep the 120-second, 1,536 MiB, 500,000,000 expansion-fuel, and 10,000,000
execution-step acceptance guards. The reduced split controls below explain an
earlier scaling defect; their speedups are not whole-book speedups.

## Current CPU attribution and copy scope

A shipping CPU profile at `84367177d` reached the authenticated
200-million-action endpoint with the original wall, memory, and execution-step
guards. The 99 Hz user-cycle capture lost no samples. List copying accounted
for 5.67% inclusive CPU and 1.01% self CPU. Expanded token delivery accounted
for 35.45% inclusive CPU; these nested percentages must not be added. This
prefix suggests a smaller remaining copying opportunity than the older
full-book profile, but it does not establish the cost of later chapters.
The raw capture and symbolized reports are in
`target/perf-tex-copy-plan/color-storage-cpu-200m/1-B/`. Concurrent builds make
this attribution evidence, not a matched runtime comparison.

A longer capture of `bf4a6a7ba` under the original guards confirms that copying
gets more expensive later: its inclusive share was 5.73%, 6.28%, 9.52%, and
8.00% in successive elapsed-time quarters, or 7.39% overall. Closure sealing
rose from 1.85% in the first quarter to 4.08% in the last. This instrumented run
timed out after 120.25 seconds and peaked at 576,428 KiB RSS; it did not reach
the same endpoint as the unprofiled fuel-limited run. Its 11K cycle samples
lost none. Raw reports and quarter boundaries are in
`target/perf-tex-copy-plan/ownership-annex-cpu-original-guards/1-B/`.

The integrated copy writer now opens one lazy rollback scope for a complete
record, including its variable annex spans and final body. Failure or unwinding
restores unpublished storage; successful publication commits the scope. A
matched CPU-10 microbenchmark measured variable-payload copying at median
552.095 ns/node before this change and 510.055 ns/node after it. The candidate
was faster in all four paired legs. Our agents paused other heavy work, but
unrelated machine activity was not audited, so the roughly 7.6% estimate is
not a machine-wide quiet measurement. The shorter all-shape run was noisy and
does not establish an improvement for fixed bodies or nested lists. Raw
samples and source/binary identities are in
`.worktrees/slot-3/target/perf-tex-copy-plan/annex-token-candidate/quiet-comparison.json`.
The 50 ns/node fixed-body target remains unmet.

The combined ownership and writer checkpoint `bf4a6a7ba` passed all seven
native/quality stages, all 93 previously passing arXiv PDF comparisons, and
the eight LaTeX/pdfLaTeX cases across TeX Live 2023–2026. Its frozen shipping
binary and receipts are under `target/perf-tex-copy-plan/ownership-annex-*`.
On the preceding ownership checkpoint `ce1f13bb8`, e-TRIP and Gentle passed;
TRIP retained its known paragraph-tracing mismatch at log byte 166256, while
normalized DVI, all 432 geometry events, and all 22 command events matched.
These results cover the inline generated-line transfer, not the unfinished
nested-child and alignment ownership extensions described in
[Generated box ownership](generated_box_ownership.md).

## Book identity and workload

The original row receipt is
`target/arxiv-pdf-wave3-final/rows/2606.24937/result.json`. It records the
command, source archive, selected 2025 distribution and formats, working
directories, and output hashes. The source archive SHA-256 is
`3a052603f4914ef4226b2ad7401ae56c7e64919a53c5a422061329c555f2ebbe`;
the materialized `book.tex` SHA-256 is
`4e120d67196807d37974163f1bc8c5526f2b2df55a6c5ad0812378d3dfc9d9b2`.
The Umber format SHA-256 is
`57f9889486c66afd49e3776877dd0b53766d6d9bb79091053db58feb108d632e`;
the distribution manifest SHA-256 is
`b5358fc48c9de49b44b39ab4ae255d7ba9ea3c22575b79a68ff852673b52c9b1`.
The original frozen Umber test-profile binary is
`target/parity-wave3/umber-final-8dd33323c` (SHA-256
`fb3d9e353b42e526c78c272da0bbed3642c597be3fe7581ab72c6e5ab7233cb5`).

The materialized source contains 74 files. `book.tex` has 1,607,085 bytes,
29,487 lines, 35 chapter commands, 309 sections, 705 subsections, 3,415 list
items, 187 listings with 240,161 body bytes, 647 instances of five breakable
`tcolorbox` environments, 72 image inclusions, 753 `\cite` commands, and
1,456 labels. Breakable boxes occur across all four source-line quartiles
(218, 176, 103, 150). The pinned pdfTeX oracle completed 588 pages in
30.30 seconds elapsed and 30.09 seconds user CPU with 73,788 KiB maximum RSS.
Its log has 10,424 PDF destinations and 14,969 PDF objects. This is a large,
mixed LaTeX workload; the source counts alone do not identify a loop or
scaling owner.

## Measured split owner and correction

The book's selected `tcbbreakable.code.tex` calls `\vsplit` while assembling
breakable boxes. Before the correction, `normalize_split_infinite_shrink`
appended every unchanged source node as a separate one-element page-list
range, even if there was no offending glue. After the first box or rule,
`prune_page_top_list_with_discards` likewise appended every remaining node
individually to the retained list; it also appended each discarded prefix
node individually to a separate list. Each same-page append could enter
`PageMaterialArena::append_reencoded_chunk_range`, which walks predecessor
chunks from tail to head before checking whether the selected range overlaps
the chunk. `ForkArena::admitted_previous_chunk` checks a fixed two-entry
lineage array; its cost came from repeated calls, not a growing lineage
search. With L retained nodes and a proportional number of chunks, L
one-node appends can traverse O(L²) chunks per prune. Repeatedly splitting
progressively smaller remainders can therefore yield O(N³) traversal. These
are source-derived bounds, not measured iteration counts.

The current-base N=800 release profile collected 2,039 cycles samples with
none lost. `append_reencoded_chunk_range` held 78.47% inclusive and 49.81%
self; `admitted_previous_chunk` held 21.98% inclusive and 21.88% self.
Durable-to-Page recursive copying held only 1.78% inclusive on this reduced
input. The profile is
`.worktrees/slot-2/target/book-timeout-profile/reduced/split-800.perf`, with
its symbolized `split-800-report.txt`. The full-book baseline release profile
separately showed growing recursive node-copy shares: Durable-to-Page
`copy_list_recursive` rose from 7.23% to 13.19% inclusive between its first
and last quarters, and Page-to-Durable rose from 3.26% to 6.94% inclusive.
Those overlapping full-book shares do not make register-copy elimination the
split-control fix; their book-level cost still needs attribution. The report
is `.worktrees/slot-2/target/book-timeout-profile/release-children-report.txt`.

The correction in `530552805` returns the original page-list identity when
there is no infinite-shrink glue. Otherwise it appends each maximal unchanged
range once and replaces only offending glue. Remainder pruning now collects
contiguous retained and discarded ranges separately, appends each range once,
and preserves the inserted split-top glue and source order. This follows the
TeX82 split rule: section 976 normalizes infinite shrink, and section 977
extracts the prefix and replaces the source with its pruned remainder. The
change leaves box-register consumption, page ownership, and the lower-level
range traversal contract as separate concerns.

## Matched reduced-workload result

Both Umber binaries used the same generated `split-N.tex` inputs, optimized
release configuration, and CPU 10. Each generated input puts N zero-width
1 pt-high rule boxes in register zero, separated by zero-point vertical
skips, then performs N repetitions of `\setbox1=\vsplit0 to 1pt`, counting
whether the source box is nonvoid before each split. This synthetic workload
is separate from the original book. Every run exited zero and reported exactly
N nonvoid source-box observations. The baseline release binary SHA-256 is
`45f6736b4dd09893c5ace1e62c16e581dae827e207194e2c9e24b4da46c37323`;
the candidate SHA-256 is
`74834fe6eedb39305ec888ca256866f7a768092297e582ddff8eeb20cfdd74c0`.
The N=800 input SHA-256 is
`07456e9d133a7511924fbaa7c67d4403bf3d79fd6e0a66e660c48925c9357970`.

| Splits | Baseline user CPU | Candidate user CPU | Speedup |
| -----: | ----------------: | -----------------: | ------: |
|    100 |           0.098 s |            0.071 s |   1.38× |
|    200 |           0.329 s |            0.161 s |   2.04× |
|    400 |           1.596 s |            0.486 s |   3.28× |
|    800 |          10.043 s |            1.900 s |   5.29× |

Sources and controls are under
`.worktrees/slot-1/target/book-vsplit-scaling/`; matched timing and DVI
artifacts are there and under `.worktrees/slot-2/target/book-timeout-profile/reduced/`.
The candidate N=800 capture has 387 cycles samples with none lost. The old
`append_reencoded_chunk_range` hotspot is absent from its flat top. The
candidate capture is `candidate-split-800.perf` beside its symbolized report
in the latter directory. The reduced speedup establishes that the targeted
path improved; it does not measure its share of the complete book.

## Original-book and regression boundaries

Both the frozen original test-profile binary and the baseline release binary
exited 124 at the 120-second wall guard on the full book. The candidate
release binary also exited 124 after 120.29 seconds elapsed, 114.53 seconds
user CPU, and 1,059,984 KiB maximum RSS. None produced a completed book PDF
or input-record artifact. Its exact command and guard output are in
`.worktrees/slot-2/target/book-timeout-profile/candidate-book.{time,stderr}`.
Do not infer a whole-book speedup from the reduced control.

For an exact internal-work boundary, a fresh copy of the original 74 source
files ran on CPU 10 with the same pinned format, distribution, offline mode,
PDF output request, 120-second wall and 1,536 MiB RSS guards, and 10,000,000
execution-step cap. Only expansion fuel changed. The baseline release run
exited at each requested fuel cap:

| Fuel actions | User CPU | Elapsed | Maximum RSS |
| -----------: | -------: | ------: | ----------: |
|    1,000,000 |   0.70 s |  1.00 s | 160,468 KiB |
|    5,000,000 |   1.15 s |  1.41 s | 160,336 KiB |
|   20,000,000 |   3.33 s |  3.80 s | 192,464 KiB |
|   50,000,000 |   9.30 s | 10.06 s | 250,448 KiB |
|  100,000,000 |  24.50 s | 25.98 s | 461,392 KiB |
|  200,000,000 |  51.91 s | 54.54 s | 636,368 KiB |

Artifacts are under
`.worktrees/slot-3/target/book-timeout-audit/fuel-endpoints/fuel-N/`.
A balanced serial A/B–B/A comparison then ran four fresh, verified
source copies on CPU 10 to exactly 200 million expansion-fuel actions.
Every run exited 1 at that cap without acquiring additional resources:

| Order | Binary | User CPU | Elapsed | Maximum RSS |
| ----: | :----- | -------: | ------: | ----------: |
|     1 | Base   |  51.86 s | 54.47 s | 636,060 KiB |
|     2 | Fix    |  51.86 s | 54.42 s | 636,188 KiB |
|     3 | Fix    |  51.68 s | 54.36 s | 636,316 KiB |
|     4 | Base   |  52.05 s | 54.84 s | 636,700 KiB |

Baseline median user CPU was 51.955 seconds and candidate median was 51.77
seconds, a 0.36% difference within run variation. The split fix did not
measurably improve the book's first 200 million fuel actions. Commands,
source receipts, and time records are under
`.worktrees/slot-2/target/book-timeout-profile/ab_ba_200m/`. An earlier
candidate-only 200-million run overlapped the eight-row cohort and is not
used for this comparison. Fuel exhaustion is a diagnostic boundary, not an
acceptance run. The baseline's marginal CPU per million actions was about
0.30 seconds over 50–100 million and 0.27 seconds over 100–200 million;
those endpoints did not show continuing acceleration in the later span or
identify a retained-memory owner.

Eight other original `tcolorbox` rows compiled with the candidate
under their unchanged guards. All eight exited zero and their rendered pages
and extracted text compared `equal` against the saved independent references.
The IDs and per-row evidence are in
`.worktrees/slot-2/target/book-timeout-tcolorbox-candidate/summary.json`.
The implementation's `scripts/check-and-test.sh` verdict was `PASS`: seven
stages passed with zero failures, blocks, or coverage reductions; its log is
`.worktrees/slot-1/target/book-vsplit-scaling/check-and-test.log`.

## Excluded candidates and build interpretation

`PdfState::define_destination` includes two linear searches over destination
records. A generated control compared 1,000, 5,000, and 10,000 named
`\pdfdest` commands with the same numbers of `\pdfsavepos` commands, shipping
100 commands per page. On the original frozen binary, the respective user-CPU
pairs were 1.10/1.04, 1.81/1.29, and 3.37/1.66 seconds. The 10,000-command
difference was 1.71 seconds, including all destination processing and PDF
output differences, too small by itself to explain this book's timeout.
Sources, logs, `/usr/bin/time -v` records, and PDFs are in
`.worktrees/slot-3/target/book-timeout-audit/dest-control/`. No destination
index was changed for this row. The annex fixed-array range walker also
starts at a fixed record tail kept within one logical chunk; its sampled
work is per-node bounded traversal, not a scan from the global arena origin.

`[profile.test]` uses optimization level 1. Test builds unify the
`tex-state/testing` feature through dev-dependencies. The frozen original
binary contains the `RESIDENT_MACRO_BODY_READ_COUNTERS` thread-local symbol,
verified with `nm -C`; it is gated by
`#[cfg(any(test, feature = "testing"))]` in `definition_arena.rs`. Shipping
feature resolution excludes `testing`, so the release binaries above are the
appropriate production-resolution timing comparison. The counter's captured
1.38% self-time share is specific to the frozen test-profile build and cannot
explain the shipping release timeout.

The unresolved boundary is the full book's dominant cost after the split
correction. Keep comparing authenticated original-source runs at equal work
and recording progress, memory, and symbolized owners before selecting another
fix or changing any guard. See [Profiling Umber](profiling.md#long-loaded-format-latex-prefixes)
for the release-resolution capture procedure.

## Runtime allocation follow-up

The integrated engine at `dcbefbebf` consumes destructive unboxes through
history-aware region transfer, skips conditionals through compact delivery,
and looks up expanded fonts before allocating projected metrics. Arena producers
now publish complete child-dependency metadata, eliminating a sealing-time
compatibility repair scan. The [allocation audit](runtime_storage_audit.md)
distinguishes these integration gaps from intentional shared payloads and
mutable dense banks.

The original book exposed an incomplete transfer preflight: every block being
moved must satisfy its annex dependency boundary, including blocks not reached
by the selected root. Corpus replay then exposed an unchanged shared-tail rollback
that unnecessarily requested exclusive mutation. Both now have ownership
regressions; no bounds or shared-mutation checks were weakened.

The production candidate SHA-256 is
`50a8f37add4696c7d9db13a2d5f2e9a4ba8736a4eba90772baada08b9b373440`.
The baseline is the prior split-fix production binary,
`74834fe6eedb39305ec888ca256866f7a768092297e582ddff8eeb20cfdd74c0`.
An isolated A/B/B/A sequence used the original source archive, the same format
and distribution, CPU 11, and the unchanged memory, time, and execution-step
guards. Each run stopped at the same 200-million expansion-fuel endpoint.
No compilation or corpus replay ran during this timing sequence.

| Run          | User CPU seconds | Elapsed seconds | Peak RSS KiB |
| ------------ | ---------------- | --------------- | ------------ |
| Baseline A1  | 52.26            | 54.96           | 636,060      |
| Candidate B1 | 46.80            | 49.27           | 635,032      |
| Candidate B2 | 46.51            | 49.01           | 635,288      |
| Baseline A2  | 51.94            | 54.54           | 635,804      |

Median user CPU fell from 52.10 to 46.655 seconds, approximately 10.5%; median
elapsed time fell from 54.75 to 49.14 seconds. This result measures a fixed book
prefix, not successful completion of the entire book. Individual unbox and
conditional changes did not establish a book speedup on their own; the combined
comparison is the timing evidence for this wave.

Receipts and the frozen candidate live under
`.worktrees/slot-3/target/perf-runtime-verified/`; `ab_ba_200m/` includes each full
command, binary and source-receipt hashes, exit status, stderr, and `/usr/bin/time`
output. The combined `scripts/check-and-test.sh` verdict was PASS: seven stages
passed, none failed or blocked, and no coverage reductions.

The fresh shipping-binary CPU profile uses `cycles:u` at 99 Hz with 8 KiB DWARF
stacks. It collected 11,898 samples with zero lost samples. The profiled process
received 99% CPU on CPU 11 while corpus workers used CPUs 0–7; it reached the
unchanged 120-second guard (114.11 user seconds, 1,111,848 KiB peak RSS).
Reports live in `full-book-profile/`, including demangled self and inclusive
views. This supersedes the earlier capture that competed with another process
on CPU 10.

The largest remaining self cost is compact raw token delivery,
`get_next_hot_into` (8.08%). Expanded token delivery accounts for 30.46%
inclusive, and page/durable copying remains visible: durable-to-page recursive
copying is 12.36% inclusive, page-to-durable 6.23%, and fixed box-annex reads
3.83% self. These overlapping inclusive costs must not be added. Realized-font
lookup and identity hashing no longer appear above the report's 0.5% threshold;
the legacy dependency-repair routine has been removed. The remaining node
copies need ownership-specific investigation, rather than treating every copy
as a compatibility path. The profile identifies CPU costs; it does not establish
a new pathological scaling cause.

The separate unprofiled run also reached the original 120-second guard:
114.56 user seconds, 120.33 elapsed seconds, and 1,139,360 KiB peak RSS
(`full-book/`). The full-book timeout remains unresolved. The 10.5% fixed-work
improvement is not evidence that the book now completes within the guard.

The final original-source replay used four workers on CPUs 0–7 and restored
all 93 previously successful papers: all 1,678 pages have exactly matching
144-dpi rendered pixels and extracted text against their immutable reference
PDFs. The binary hash in every receipt is the candidate above. Results are in
`parity-93/summary.json`; the two rollback regressions also have independent
receipts in `repro-two/`. These are rendered-output and text parity results,
not byte-identical PDFs or identical internal PDF object graphs.

## Batched delivery and admitted node reads

The next engine, `99f75a4d9`, extends the existing resident reader to discard
eligible literal runs during conditional skipping under one frame admission.
The 4,096-character regression requires one admission, while preserving exact
per-token fuel and the next unread token. Observed, traced, active-alignment,
source, brace, and control-sequence boundaries keep their canonical settlement.
The first ordinary unobserved macro in main-control preflight also activates
directly from its resolved input, using the expansion loop's shared admission.

The original book exposed an important batch boundary: a returned parameter
marker has already advanced its replacement cursor. Discarding that transition
loses the argument, including any conditional delimiters inside it. The fixed
reader continues through the existing charged parameter transition; scalar and
batched reads also share replay and alignment continuation. Stored and source
regressions cover delimiters in arguments, empty arguments, and nested
forwarding, comparing fuel against observed scalar delivery.

Node reads now borrow contiguous fixed annex payloads after ordinary root and
publication validation. Recursive copies collect child dependency floors while
rewriting those children, instead of decoding the destination record again.
Borrowed traversal reuses its admitted physical blocks. Forward walks use
short-lived chunk-coordinate scratch with bounded Rust-stack usage; a long-chain
test on a 128 KiB thread stack guards against recursive stack growth. Single-chunk
walks and order-independent reverse validity checks remain allocation-free.

The shipping binary SHA-256 is
`da06fc9ad71202377133c74a50eb6f861309b7805a9483fa4652c6d75c55f4b7`.
Its predecessor is the `50a8f37a` binary above. The final full native suite and
quality gate report seven stages passed, zero failed or blocked, and no coverage
reductions. Original-source replay preserves all 93 papers and 1,678 pages in
rendered pixels and extracted text. Eight additional LaTeX/pdfLaTeX cases also
match across TeX Live 2023–2026. Replay authenticates the original source archive,
reference PDF, and format hashes before running.

The full book still reaches the unchanged 120-second guard (114.59 user seconds,
120.35 elapsed seconds, 1,225,108 KiB peak RSS). The fresh `cycles:u` profile
collected 11,891 samples with zero lost samples and 99% process CPU. Expanded
token delivery remains the largest named family at 28.67% inclusive; conditional
skipping is 4.85%, durable-to-page copying 9.28%, and page-to-durable copying
5.28%. These inclusive shares overlap. Inlining and the amount of work reached
within a fixed time also change sample shares, so they are not speedup estimates.

Frozen binaries, commands, hashes, comparisons, gate logs, and profiles live under
`.worktrees/slot-2/target/perf-big-cuts/`. The successful corpus results are in
`parity-93/` and `representatives/`; `full-book/` and `full-book-profile/` retain
the unchanged-guard runs. The earlier `ab_ba_200m/` candidate failed before the
fuel endpoint and is not performance evidence. The final paired run validates
the actual fuel-exhaustion diagnostic, not just the process exit code.

The final isolated A/B/B/A comparison used CPU 11 after builds and corpus runs
finished. Every run reached the same 200,000,000-action fuel endpoint on the
original book inputs and format.

| Run          | User CPU seconds | Elapsed seconds | Peak RSS KiB |
| ------------ | ---------------- | --------------- | ------------ |
| Baseline A1  | 46.28            | 48.70           | 635,416      |
| Candidate B1 | 44.20            | 46.68           | 635,156      |
| Candidate B2 | 43.51            | 45.93           | 635,032      |
| Baseline A2  | 46.23            | 48.66           | 635,028      |

Median user CPU fell from 46.255 to 43.855 seconds, a 5.2% reduction;
median elapsed time fell from 48.68 to 46.305 seconds, a 4.9% reduction.
Peak memory at this fixed-work endpoint is effectively unchanged. This is an
additional improvement over the preceding engine, measured on a book prefix;
it does not establish full-book completion or attribute gains to individual
changes. Receipts are in `verified_ab_ba_200m/` under the artifact directory
above. The remaining dominant work is token expansion and node copying.

## Packing diagnostic context

The current write path already renders an input-stack context only when write
expansion is unbalanced. A page must still detach context for errors discovered
after its input borrow ends. The previous book profile instead attributes most
`render_context_for_levels` samples to ordinary box packing, including the
`BoxEndGroup` paths. TeX82 §§660–661 and 674 use only the current line, packing
start line, and output-routine flag for those reports; they do not invoke
`show_context`.

Packing now accepts a scalar `PackDiagnosticContext`, and command-side box
completion constructs it without rendering input. Callers that already need a
full error context project only these scalars into packing. Existing exact
packing-report and reference-diagnostic tests remain the behavior authority.
Shipout normalization also clones the detached open context only for an
`OpenOut` whatsit, instead of cloning it before inspecting every whatsit.
These changes have not yet been assigned a measured runtime saving.

## Node-pool retention and physical compaction

Sealing a short logical chunk now returns its unused physical tail to the
64 KiB superblock. This applies to both packed annex words and optional node
slots. Logical keys remain stable. If rollback must reopen a tail after later
allocations, packed words relocate within the same owner; optional values move
with `Option::take`, preserving exactly-once destruction. Cached admitted
physical positions refresh after relocation, release, transfer, or truncation.
The [node ownership contract](node_region_ownership.md) defines these rules;
this is exclusive storage compaction, not sharing or copy-on-write.

Profiling-feature builds ran the authenticated original book and 2025
format/distribution at fixed fuel endpoints. Both runs kept the 120-second,
1,536 MiB, 10,000,000-step, and two-second termination guards. Only expansion
fuel changed, and every run ended with the exact requested fuel-exhaustion
diagnostic. The comparison isolates physical compaction from the preceding
lazy annex-boundary implementation; it does not measure shipping latency.

| Build                                      | Fuel actions | Peak RSS KiB | Fresh node blocks | Fresh annex blocks |
| ------------------------------------------ | ------------ | ------------ | ----------------- | ------------------ |
| Before physical compaction (`f031f9662`)   | 100 million  | 628,904      | 1,593             | 3,051              |
| Before physical compaction (`f031f9662`)   | 200 million  | 698,024      | 1,593             | 3,051              |
| Both storage lanes compacted (`7b9f40dbd`) | 100 million  | 336,416      | 112               | 229                |
| Both storage lanes compacted (`7b9f40dbd`) | 200 million  | 405,028      | 112               | 229                |

Peak RSS falls by about 286 MiB at either endpoint. The compacted pool's 341
fresh 64 KiB blocks total 21.3125 MiB of backing, and that high-water remains
flat as fuel doubles. Owner censuses sampled at checkpoint and output
boundaries report peaks of 37/73 live node/annex blocks at 100 million fuel
and 45/78 at 200 million. These samples are not snapshots taken at the instant
of fuel exhaustion. The final zero-live-block gauge is printed after the
engine drops and cannot establish its live ownership at that boundary.

An empty superblock keeps its allocation in the pool's vacant list for reuse
until the pool drops. Retirement removes the semantic owner; it does not
immediately return backing to the system allocator. Fixed-live-set churn tests
verify bounded reuse, while rollback tests cover same-block and cross-block
relocation, stale cursors, and non-Copy values. These measurements support
reusing retired blocks rather than adding an eager-release policy.

RSS still rises by 68,612 KiB between the compacted runs without new node-pool
superblocks. These counters do not attribute that remaining growth. No
4.7 GiB full-book memory result was reproduced, so this audit does not claim
to have explained that figure.

Receipts are under `target/perf-tex-copy-plan/optional-compact-retention/` and
`optional-compact-retention-comparison.json`. The compacted profiling binary
SHA-256 is
`4486846d5c5dedbfc9072f611a7c79bcea73b645144b6dbcfb3fe3907ed62669`.

## Heap attribution beyond superblock backing

Heaptrack 1.5.0 ran the same compacted profiling binary at 100 and 200 million
fuel actions. Both runs reached the exact fuel boundary under the original
120-second and 1,536 MiB guards. Instrumentation and concurrent builds make
these diagnostic runs unsuitable for latency comparisons. Heaptrack reports
390,278,632 and 467,340,396 bytes allocated at the respective global heap peaks.
These are requested live heap bytes, distinct from resident pages and from
memory sampled at the exact fuel boundary.

The peak flamegraph attributes the following disjoint allocation paths. A
stack enters the first matching category in the order shown; remaining stacks
are grouped as other. Values use decimal MB.

| Allocation path                      | 100-million run peak | 200-million run peak |
| ------------------------------------ | -------------------- | -------------------- |
| PDF color-stack history and payloads | 14.20                | 28.57                |
| Render provenance                    | 10.12                | 21.70                |
| Verified artifacts                   | 6.89                 | 14.63                |
| Other retained shipout output        | 13.86                | 28.40                |
| Durable token lists                  | 16.56                | 24.90                |
| Definition storage                   | 38.00                | 45.63                |
| Node-pool chunk metadata             | 44.05                | 44.05                |
| Source provenance                    | 12.58                | 12.58                |
| Other                                | 234.02               | 246.87               |

The pool's metadata allocation is separate from its 21.3 MiB of superblock
backing, and also stays flat across these two runs. Output, provenance,
tokens, and PDF history explain much of the growing allocated heap. These
attributions do not prove that the retained data is unnecessary.

One concrete representation cost is avoidable: color history stores a small
pair of runtime coordinates in the general PDF version enum, paying for its
largest variant on every update. The separate color index already distinguishes
that value family. [PDF version storage](pdf_version_index.md) describes moving
these values into a dedicated compact arena while retaining every historical
root and the existing candidate settlement rules.

Raw heaptrack traces, demangled peak reports, peak flamegraphs, exact commands,
and input/binary receipts live in
`target/perf-tex-copy-plan/remaining-memory-heaptrack/`. The four allocations
left after complete teardown total 632 bytes; that end-of-process result does
not explain which histories must remain live during execution.

### Compact color-history measurement

A matched 100-million-action heaptrack run compared `f818c6b37` with
`84367177d`, which adds the dedicated color-value arena. Both reached the
exact fuel limit with the original timeout and memory guards. Peak requested
heap fell from 390,278,616 to 379,710,992 bytes, a reduction of 10.1 MiB.
Allocations attributed to `apply_color_stack` fell from 14,204,890 to
3,456,986 bytes. The remaining color allocation includes persistent lookup
history, pushed values, and payload bytes; those ownership rules did not
change. These figures describe each run's global heap peak and do not establish
a runtime improvement.

The full native/quality gate passed all seven stages with no failures, blocked
stages, or coverage reductions. The color tests cover independent page/form
stacks, interleaved general PDF versions, candidate acceptance and rejection,
and subsequent checkpoint rollback. Profile receipts, binary hashes, and
exact commands are in `target/perf-tex-copy-plan/color-storage-heap-comparison.json`,
with raw traces under `color-baseline-heaptrack/` and `color-candidate-heaptrack/`.

## Shipping comparison at the same book prefix

A separate quiet A/B/B/A run compared the earlier `99f75a4d9` shipping binary
with integrated `7b9f40dbd`. It used CPU 11, the same authenticated source,
format, distribution, and 200-million-action endpoint. Builds, corpus runs,
and other performance jobs were paused. Every trial reached the exact fuel
limit; an exit code alone was not accepted as evidence.

| Run          | User CPU seconds | Elapsed seconds | Peak RSS KiB |
| ------------ | ---------------- | --------------- | ------------ |
| Baseline A1  | 46.13            | 48.68           | 635,424      |
| Candidate B1 | 44.86            | 47.21           | 405,148      |
| Candidate B2 | 43.51            | 45.68           | 405,148      |
| Baseline A2  | 44.05            | 46.43           | 634,784      |

Median user CPU falls from 45.09 to 44.185 seconds (2.0%); median elapsed time
falls from 47.555 to 46.445 seconds (2.3%). Median RSS falls by about 225 MiB.
The runtime difference is small compared with the memory reduction, and the
trials show variation. This prefix result does not demonstrate a 20–25%
whole-book speedup or completed book output. It includes the integrated box,
copy, and allocator changes preceding source-admission caching; it cannot
assign the runtime difference to one change.

The candidate passed the full native/quality gate (seven stages, no failures,
blocked stages, or coverage reductions), all 93 previously passing arXiv PDF
comparisons, and eight LaTeX/pdfLaTeX cases across TeX Live 2023–2026. PDF
comparison means equal rendered pixels and extracted text. These results do
not cover unfinished generated-box transfer work or establish an exact TRIP
log pass; the known paragraph-scoring log mismatch remains separate.

Commands, source and binary hashes, exact diagnostics, and timing receipts are
in `target/perf-tex-copy-plan/optional-compact-abba-200m/`. The shipping candidate
SHA-256 is
`365e89f2ed9e62450f756db010b96da39d51db014c41e0a6ceef39b6dae45067`.

The same shipping binary still reaches the original full-book timeout: exit
124 after 120.17 seconds, with the original 500-million-fuel and 1,536 MiB
limits. The receipt is
`target/perf-tex-copy-plan/optional-compact-original-book/summary.json`.
