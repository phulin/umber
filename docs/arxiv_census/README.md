# Pinned arXiv 100-document census

This directory preserves two complete, machine-readable 100-row captures of
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
