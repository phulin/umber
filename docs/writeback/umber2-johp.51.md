# umber2-johp.51 — expanded internal-value token-list splice

Authority: TeX82 `tex.web` §§26--27 (`scan_toks` and `the_toks`) and §424
(`scan_something_internal`), with the pinned Gentle semantic trace
`1c45c90cc0e19e119ddd45dbdf618f7eb557d2eeeea21e15f96886d5065f049` at
events 102090--102106.

In an expanded token-list scan, `\the` expands only its internal-value target
and appends the rendered token list directly. `\spacefactor` is that internal
integer when the active executor list is horizontal. Command replay supplies
the current value through its bounded host capability, refreshed for each
operation; it remains outside durable command state and snapshots. The direct
splice therefore preserves the collector's one-step expansion and brace-depth
rules.
