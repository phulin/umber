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
