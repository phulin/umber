# Long-book PDF performance diagnosis

The recent-arXiv PDF row `2606.24937` remains a performance diagnosis in
progress. Its complete, unmodified `book.tex` compiles with the pinned pdfTeX
oracle, while the frozen Umber test-profile binary reaches the ordinary
120-second wall guard. The archived failure alone does not identify a semantic
loop, a scaling owner, or the behavior of a release build. Keep the
120-second, 1,536 MiB, 500,000,000 expansion-fuel, and 10,000,000
execution-step guards when testing this row.

## Reproduction identity and workload

The primary receipt is
`target/arxiv-pdf-wave3-final/rows/2606.24937/result.json`, with the exact
commands, source archive identity, selected 2025 distribution and formats,
working directories, and output hashes. The source archive SHA-256 is
`3a052603f4914ef4226b2ad7401ae56c7e64919a53c5a422061329c555f2ebbe`;
the materialized `book.tex` SHA-256 is
`4e120d67196807d37974163f1bc8c5526f2b2df55a6c5ad0812378d3dfc9d9b2`.
The frozen Umber binary is `target/parity-wave3/umber-final-8dd33323c`
(SHA-256 `fb3d9e353b42e526c78c272da0bbed3642c597be3fe7581ab72c6e5ab7233cb5`).
The Umber format SHA-256 is
`57f9889486c66afd49e3776877dd0b53766d6d9bb79091053db58feb108d632e`;
the distribution manifest SHA-256 is
`b5358fc48c9de49b44b39ab4ae255d7ba9ea3c22575b79a68ff852673b52c9b1`.

The book is 1,607,085 source bytes and 29,487 lines. It has 35 chapter
commands, 309 sections, 705 subsections, 3,415 list items, 187 listings with
240,161 bytes of listing body text, 647 instances of its five breakable
`tcolorbox` environments, 72 image inclusions, 753 `\cite` commands, and
1,456 labels. The breakable boxes occur throughout the four line quartiles
(218, 176, 103, 150); the listings likewise occur throughout (38, 50, 57,
42). This is a large mixed LaTeX workload, not evidence of a runaway loop.

The oracle's `reference.time` records 30.30 seconds elapsed, 30.09 seconds
user CPU, and 73,788 KiB maximum RSS. Its `reference/book.log` records 588
pages, 10,424 named PDF destinations, and 14,969 PDF objects. The reference
PDF SHA-256 is
`76d0a209515de0b3f9e6dc690ff31eb0109708d25c253a6e4a18c9c60b141ad6`.
The frozen Umber command exits 124 at the 120-second guard; `umber.log`
contains only the guard settings and timeout notice. There is no completed
Umber PDF or input-record artifact in that row. Earlier corpus receipts also
record exit 124, but do not provide a measured growth curve.

## Destination scaling control

Source inspection found two linear scans in `PdfState::define_destination`:
`reserve_destination` searches existing destination identities, then
definition finds the reserved object row. With 10,424 oracle destinations,
this is a plausible quadratic component, but source inspection cannot assign
its runtime share. A bounded control challenged that hypothesis before any
engine change.

The frozen Umber binary compiled generated LaTeX inputs with 100 explicit
zero-width boxes per shipped page. One family placed a distinct named
`\pdfdest` in each box; the matched family placed `\pdfsavepos`. Each run used
the receipt's pinned format and distribution, offline mode, and the ordinary
guards. Runs were pinned to CPU 10. The controls completed without TeX errors.

| Commands | Named destinations: user CPU | Saved positions: user CPU | Difference |
| -------: | ---------------------------: | ------------------------: | ---------: |
|    1,000 |                       1.10 s |                    1.04 s |     0.06 s |
|    5,000 |                       1.81 s |                    1.29 s |     0.52 s |
|   10,000 |                       3.37 s |                    1.66 s |     1.71 s |

The 10,000-command runs each shipped 100 pages. Generated sources, logs,
`/usr/bin/time -v` output, and PDFs are under
`.worktrees/slot-3/target/book-timeout-audit/dest-control/` as
`dest-N.{tex,log,time,pdf}` and `savepos-N.{tex,log,time,pdf}`. The measured
difference includes all destination processing and PDF output differences;
it is not a measurement of the two scans alone. It is too small to explain
the book's 120-second failure by itself. Do not promote a destination-index
change as this row's fix without a representative profile assigning it a
meaningful share.

## Original-source work endpoints

A separate shipping-resolution release binary in
`.worktrees/slot-2/target/release/umber` (SHA-256
`45f6736b4dd09893c5ace1e62c16e581dae827e207194e2c9e24b4da46c37323`)
ran fresh copies of all 74 materialized source members, with the same pinned
format, distribution, offline mode, PDF output request, and 120-second,
1,536 MiB, and 10,000,000-step guards. Only expansion fuel changed to make
exact engine-work endpoints. Each run was pinned to CPU 10, exited 1, and
reported exhaustion at precisely its requested fuel count. These diagnostic
endpoints are not corpus acceptance runs.

| Fuel actions | User CPU | Elapsed | Maximum RSS |
| -----------: | -------: | ------: | ----------: |
|    1,000,000 |   0.70 s |  1.00 s | 160,468 KiB |
|    5,000,000 |   1.15 s |  1.41 s | 160,336 KiB |
|   20,000,000 |   3.33 s |  3.80 s | 192,464 KiB |
|   50,000,000 |   9.30 s | 10.06 s | 250,448 KiB |
|  100,000,000 |  24.50 s | 25.98 s | 461,392 KiB |
|  200,000,000 |  51.91 s | 54.54 s | 636,368 KiB |

The corresponding `run.log`, `run.time`, and source copy are under
`.worktrees/slot-3/target/book-timeout-audit/fuel-endpoints/fuel-N/` in the
primary checkout. The marginal user CPU per million fuel actions is about
0.30 seconds over 50–100 million actions and 0.27 seconds over 100–200
million. These endpoints do not show continuing acceleration over the later
span. They also do not identify the operation mix, page progress, or an owner
of retained memory. Fuel exhaustion produces no completed PDF or input-record
artifact.

## Release profile boundary

The release capture's first-to-last-quarter inclusive shares rise from 7.52%
to 14.67% in `copy_record_chunk_prefix`, from 7.23% to 13.19% in the
Durable-to-Page `copy_list_recursive` specialization, and from 3.26% to
6.94% in the Page-to-Durable specialization. These call trees overlap and
must not be summed. Expanded token delivery stays in a narrower 19.51% to
17.08% band. The symbolized report is
`.worktrees/slot-2/target/book-timeout-profile/release-children-report.txt`.
This identifies recursive node copying as growing work within the captured
release run. Inclusive shares overlap; they do not establish that copying is
the dominant cause of the timeout.

The selected `tcolorbox` package's `tcbbreakable.code.tex` uses `\vsplit`
and `\unvbox` while assembling breakable boxes. Umber's
`split_vbox_register` copies a durable box into page storage, and
`replace_split_source` copies the remainder back to durable storage. TeX's
explicit `\copy` and retained rollback history can require structural
copies. The reduced profile below assigns its principal cost to same-page
range re-encoding, so a copy-frame sample alone does not justify changing
`\vsplit` register ownership.

## Reduced vertical-split owner

A repeated-`\vsplit` control with 100, 200, 400, and 800 repetitions
on the current release binary took Umber 0.13, 0.32, 1.59, and 10.05 seconds
elapsed (0.09, 0.29, 1.56, and 10.01 seconds user CPU). The pinned pdfTeX control took
0.06, 0.10, 0.15, and 0.30 seconds. A matched consuming-box control took
Umber 0.04, 0.04, 0.05, and 0.07 seconds. This establishes disproportionate growth in
Umber's split path on the reduced input; it does not quantify how much of the
full book it explains. The N=800 release run returned zero and reached
39,040 KiB maximum RSS. Its same-input, same-binary profile collected 2,039
cycles samples with none lost. It assigns 78.47%
inclusive and 49.81% self to `PageMaterialArena::append_reencoded_chunk_range`,
21.98% inclusive and 21.88% self to `ForkArena::admitted_previous_chunk`,
and only 1.78% inclusive to the Durable-to-Page `copy_list_recursive`
specialization. The control sources and runner are under
`.worktrees/slot-1/target/book-vsplit-scaling/`; the N=800 source SHA-256
begins `07456e9d`. Its matched release capture and report are under
`.worktrees/slot-2/target/book-timeout-profile/reduced/` as
`split-800.{perf,tex}` and `split-800-report.txt`.

In `normalize_split_infinite_shrink`, Umber appends every unchanged source
node as a separate one-element range, even when `vert_break` reports no
infinite-shrink glue. Each append can enter `append_reencoded_chunk_range`.
That function starts at the source tail and recursively follows predecessor
chunks to the head before testing whether the selected range overlaps a
chunk. Predecessor resolution checks a fixed two-entry lineage array, so the
lookup itself has bounded cost. The high predecessor sample share reflects
its repeated calls. Repeating a full-chain traversal for each one-node range
accounts for the measured scaling shape; the profile establishes the hot
functions, while source inspection establishes their traversal order.

The first correction boundary is the semantic identity case: when the
infinite-shrink index set is empty, return the original page-list identity.
When replacements exist, preserve order by appending each maximal unchanged
range once and inserting a replacement only for the offending glue node.
TeX82's `vsplit` (section 977, `tex.web` part [44]) changes shrink order only
for infinite-shrink glue that triggers the split diagnostic. This design
keeps marks, break index, remainder, and register ownership in their existing
owners. Recheck the matched control and full book after implementation;
`prune_page_top_list_with_discards` has a separate per-node append path that
may still matter. A range traversal fix requires its own dependency and
rollback audit, since admitted predecessors can carry retained history.
The annex fixed-array range walker, by contrast, begins at the fixed record
tail; fixed records are kept inside one logical chunk. Its sampled work is
per-node bounded traversal, not a scan from the global arena origin.

## Build and profile interpretation

`[profile.test]` uses optimization level 1. Test builds unify the
`tex-state/testing` feature through dev-dependencies. The frozen binary
contains the `RESIDENT_MACRO_BODY_READ_COUNTERS` thread-local symbol, as
verified with `nm -C`. The counter is gated by
`#[cfg(any(test, feature = "testing"))]` in `definition_arena.rs` and updates
on resident macro-body reads. The shipping feature resolution excludes
`testing`; the release performance comparison must use a separately built
shipping binary rather than treating the frozen test-profile timing as a
release timing. This also narrows the counter's relevance to the captured
test-profile build.

For the complete book, profile with the authenticated input and guards, record
the binary hash and feature resolution, and use matched engine-work endpoints
when comparing internal scaling. A wall timeout is a failure observation,
not a work boundary. Record page or source progress and RSS at bounded
endpoints before deciding whether elapsed time is steady per page or grows
with retained state. Compare production and profiling-feature builds
separately, as specified in [Profiling Umber](profiling.md#long-loaded-format-latex-prefixes).
The reduced control identifies a same-page range-reencoding owner. Its
contribution to the complete book and the outcome of the guarded row after a
principled correction remain to be measured.
