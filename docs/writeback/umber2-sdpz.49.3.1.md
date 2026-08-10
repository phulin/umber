# umber2-sdpz.49.3.1: Suffix-Local TeX `unsave` Accounting

## Reproduction identity

The exact recent-arXiv row was reproduced from git `881b7e3ffb9c6c892281639612248831c22a6b43` with source `neurips_2026.tex` SHA-256 `164d46b605e345a852dfe144d2536a78884694f77f7e379e69dcacde86bcfde5`, archive SHA-256 `d60f34186e584c6cbb046e5ad8f5c1118b0ca3f6a637574097127300acdc68be`, schema-11 pdfLaTeX format SHA-256 `0cfb18d9b9f4548fab57f3e003b1d9c886a9b3bbc633781e67430ae2f0669242`, and explicit 2026-03-01 distribution root SHA-256 `43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b`. The unchanged offline limits were 100,000,000 canonical actions, 120 seconds, and 1,536 MiB. The baseline status was 124; `target/umber2-sdpz.49.3.1/row-881b/offline/engine.log` has SHA-256 `499a3f14fdb86b5687540391cb961952fbe9ab1c00de53c49b416cda6c352ae6`.

The exhaustive command tracer reported zero gating or advisory divergences. Gentle and Story locators completed at 97 artifacts/263,424 DVI bytes and one page/680 DVI bytes respectively. `umber2-sdpz.58` owned the already-closed `\noexpand`/`\endcsname` classification front; this row instead remained inside state-accounting work and was disjoint before editing.

## Earliest growth transition

A 30-second `perf` capture identified `main_memory_usage_inner` as the largest initial Umber self owner at 12.97%, followed by the Env root walk at 4.46%, `SaveStackProjection::push_undo` at 3.12%, and journal truncation/reallocation work. A negative control that removed only the two group-exit memory observations made the focused high-water test fail and left the row at status 124, so that deletion was rejected.

The bounded journal probe then exposed the first superlinear transition. Projection rebuild 1 retained 16 entries; rebuilds 1,024 and 2,048 replayed 36,663 and 48,576 retained entries respectively. The evidence is `target/umber2-sdpz.49.3.1/journal-probe/engine.log`, SHA-256 `7f0c720eeaf8c91cd54dca8bf81d1193230daba65779578ca00db108ce2e6d19`. Package-prefix reductions showed that `tcolorbox` and each preceding batch completed independently under a 30-million-action/30-second bound, while their combined retained state crossed it. This identifies cumulative bookkeeping rather than a package- or paper-specific semantic loop.

## Canonical owner and fix

TeX82 §§273--280 append save-stack records when definitions and group boundaries are encountered. Section 283's `unsave` consumes the current suffix in reverse; it neither replays retained level-zero definitions nor scans the whole live heap. Sections 125--127 move `lo_mem_max` and `hi_mem_min` at allocator events, while §1334 reports the resulting persistent coordinate. Freeing values during §283 therefore cannot be the owner of a fresh full-closure allocation scan.

The journal projection now records one inverse delta beside each appended entry. Truncation reverses only the removed suffix, including restoration of local-save eligibility removed by a later global assignment. Group exit no longer performs the full main-memory closure scan; scanner-owned token words and node-list construction remain the canonical allocator-event observations. Focused controls cover nested rollback, global/local eligibility, a 4,096-entry retained prefix with exactly two suffix reversals, allocator-event high water, and an unowned immutable-list negative case. Exact compatibility TRIP and the official two-phase e-TRIP artifact gate both remain unchanged and green.

## Row disposition

The fixed binary before commit had SHA-256 `c40d800d2efd3075cd2999837f3aad3671c633b2eb328645c72a7dfb613b96b7`. The exact row advanced far enough to request and authenticate three additional distribution resources; `target/umber2-sdpz.49.3.1/fixed-warm-2/engine.log` has SHA-256 `2269880400b5ca348e2ff2a7c5de160e02613d54b7cfd1943d907b4b79bbb96b`. After that closure was warmed, the offline row still reached the 120-second guard, now without the journal rebuild owner. The post-fix 9,000-sample capture lost no samples and instead leads through command delivery and checkpoint/state-hash projection; it is `target/umber2-sdpz.49.3.1/fix2-profile/perf.data`, SHA-256 `829cb24049ff4454ad88f11348482ba8b6a168406898c6005bfa06dad63fe578`.

That residual, separately owned runtime front is recorded observed-only as `umber2-sdpz.49.3.2`. No guard, distribution identity, paper source, or comparison contract changed.
