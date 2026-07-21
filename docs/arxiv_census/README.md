# Pinned arXiv 100-document census

This directory preserves complete, machine-readable 100-row captures of
the corpus selected by `scripts/pdftex-arxiv-sample-100.tsv`. Each capture has:

- `metadata.json`: engine, format, manifest, timeout, concurrency, and (for the
  integrated capture) per-row engine identity counts;
- `results.jsonl`: one row per manifest document, including the selected
  entrypoint, exact command, source hash, result, elapsed time, peak RSS, log
  hash, terminal diagnostic, and assigned cluster;
- `clusters.json`: an exact-accounting summary whose cluster counts total 100.

## Baseline

`baseline/` is the isolated fresh run at selector commit
`33965c86785630aba4eded136c80f84c1710fd8e`, using four workers and a 240-second
per-document timeout. It is the pre-integration reference. Its 100 rows include
26 font-map/program failures, 21 frozen-token panics, 16 provisional timeouts,
seven impossible source bundles, and the smaller semantic clusters recorded in
`clusters.json`.

## Integrated capture

`integrated/` is the completed current-state capture after the relevant parity
fixes were integrated. The selector branch is
`8c20cdad596ce633d78045314d3aa3611717d10d`; `metadata.json` records the exact
commit and binary hash attached to every row because the resumable census spans
two earlier binary identities (three rows total). The run used a 120-second
per-document timeout. The final three targeted rows used one worker and a
1.5-GiB RSS guard; the exact per-row peak remains in `results.jsonl`.

The 14 rows in
`provisional_expansion_work_limit_umber2_65ku_36` are deliberately not treated
as document root causes. The same 250,000-step failure for `2606.13617`
reproduced at both `1de2ab31` and its parent `765032eb`, exonerating the later
math-recovery and token-register changes. Beads issue `umber2-65ku.36` owns the
work-accounting investigation and its exact A/B evidence. The 22 timeouts and
one RSS-limited row are likewise provisional until bounded reruns follow that
fix.

The seven source-side impossibilities are stable corpus facts and are explained
in `docs/arxiv_corpus.md`. All engine and hosted-snapshot clusters are tracked
as children of Beads epic `umber2-65ku`; they are observations, not permission
to patch paper sources or special-case the corpus.

## 2026-07-21 completion audit

`final-20260721/` is the fresh exact-100-row audit at commit `05ef9919`. It uses
the coherent regenerated 2026-03-01 distribution and format with serial guarded
children capped at 100,000,000 engine fuel, 1,536 MiB RSS, and 120 seconds.
Fifty rows produce PDFs. Corrected reference accounting identifies 79 clean
rows: 41 produce PDFs and all remaining 38 link exactly once to focused Beads
blockers `umber2-65ku.61` through `.70`. The other 21 rows retain the independent
reference-failure classifications recorded in `docs/arxiv_corpus.md` and the
epic audit.

The regenerated local distribution proves `1204.5690` completes through the
generated `pdftex.map` path. It does not prove hosted reproducibility: the
production immutable pin still lacks that map, and blocked issue
`umber2-65ku.19` owns publication, public verification, and pin rotation.

The requested offline replay was stopped after 74 rows because of its runtime;
all completed rows matched the warm engine/finalizer outcome exactly. The
machine receipt `offline-partial.json` records counts, the partial-summary
digest, and phase timing. The 6,125-second warm run spent 3,416 seconds in
engine subprocesses and 2,676 seconds in repeated PDF-finalizer subprocesses;
resource waits account for 300.5 seconds inside engine time and orchestration
for about 33 seconds. Rebuilding, cache acquisition, and orchestration are
therefore not the dominant cost.

The census runner now transfers accepted state directly into PDF finalization
in the same guarded process and publishes each completed row atomically. The
old 2,676-second `finalizer` bucket was a second full compile plus real PDF
lowering, not finalization alone. The first single-pass `1609.01918` diagnostic
still used an unoptimized development binary and took 77.5 seconds. The
repository's intended `profile.test`/`cargo run-dev` artifact took 9.57 seconds
before PDF fixes and 3.89 seconds afterward, versus 17 plus 77 seconds from the
historical engine and finalizer log timestamps. This is a 24.2x reduction from
the original two-process row and preserves its accepted/complete outcome.

The optimized pre-fix PDF build spent 2.03 seconds lowering virtual fonts and
3.92 seconds collecting font usage. Virtual-character expansion cloned a whole
VF program and packet for every character, then usage collection rescanned all
PDF font operations for every expanded glyph run. Shared immutable program
ownership, borrowed packets, cached live-to-artifact font identities, adjacent
leaf-run coalescing, and one metadata projection per distinct font reduce those
phases to 0.242 seconds and 0.005 seconds. That VF phase processes 51,055
characters and 52,785 packet commands with only 25 program lookups and 38
local-font constructions; the complete PDF build is 0.473 seconds, including
0.109 seconds to serialize 206 objects into a 0.996 MB PDF. Scaling the
historical 32 failed first passes by the measured 4.4x optimized-engine ratio
and the 68 accepted combined passes by the measured 19.9x row ratio projects
roughly 9 minutes for 100 rows, versus 102 minutes.

A non-VF accepted paper (`2402.06118`) confirms that the remaining work follows
its inputs rather than a hidden per-glyph finalizer loop: 9,107 positioned
events pass through VF lowering unchanged in 0.048 milliseconds. Before the
image fix, 0.931 of its 1.05-second PDF build imported three distinct RGBA PNGs
totaling 7.64 MB: IDAT extraction/copy took 0.004 seconds, inflate 0.079,
unfilter/split 0.231, and re-encoding 0.615. Splitting the still-filtered PNG
rows into color and alpha predictors and using the fast bounded encoder reduces
transform and encode to 0.124 and 0.099 seconds. Image import is 0.312 seconds
and the complete PDF build is 0.429 seconds; validation and serialization take
0.10 and 1.67 milliseconds, and `pdfinfo` validates the resulting 18-page,
8.37 MB PDF.

JPEG data and opaque PNG IDAT streams were already validated and passed through
without decode/re-encode. Image source resolution now caches an immutable
`PdfExternalImageSource` by the complete request, so a repeated request does not
reread, rehash, or reparse its shared bytes. Detached lowering also reuses one
raster XObject and soft-mask pair by content identity plus metadata, including
across pages and forms. Distinct PDF-page image allocations intentionally remain
distinct because their form/group object identity is observable; repeated
references to one allocation already share its imported form. All PNG decoded-
length, filter-byte, dimension, imported-stream, and aggregate PDF bounds remain
in force.
Completed rows resume by immutable input and artifact identity. Offline
reproducibility is attested without recompilation by rehashing those receipts,
based on the acquisition layer's authenticated-before-use cache invariant; an
independent semantic replay can still be requested separately when needed.

The guarded format-load probe used the exact audit image at
`target/pdflatex-format/pdflatex.fmt` (SHA-256
`f640624c160500d6faafd88be3c381e94390e7edb4a547d82a4350eef73a96f4`). A
minimal formatted article emitted the pinned `LaTeX2e <2025-11-01>` and L3
banner. It took 5.13 seconds with the invalid development-profile census build
and 1.10 seconds with the required optimized artifact. In the optimized run,
format reading took 0.26 milliseconds and restore took 0.13 milliseconds;
engine work was 0.133 seconds and resource wait was 0.052 seconds. Invalid image bytes were rejected as a
truncated Umber format, while omitting `--format` completed in 0.01 seconds
with undefined `\documentclass` and `\begin`. Census rows therefore deserialize
the pinned format rather than rebuilding LaTeX or running initex.
