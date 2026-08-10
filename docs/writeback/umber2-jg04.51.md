# umber2-jg04.51: Write-Scan One-Word Allocator Extent

The exhaustive canonical tracer is clean. After transient typed node-list
sampling advanced official e-TRIP to `2999` words, the pinned Web2C high-memory
allocator was watched directly during the format-loaded phase. `hi_mem_min`
reaches `248704` at e-TRIP line 394 with `dyn_used=1295`, `var_used=316`, and
an empty `avail` list. The allocating stack is `get_avail` through `scan_toks`
and `write_out` while an immediate `\write` is being expanded.

TeX82 §200 establishes the one-word reference-count head for a token list.
Sections 357 and 390 establish the ownership of macro-argument token nodes.
Section 1370 keeps the original write list live, inserts the artificial left
brace, right brace, and `endwrite` nodes, and scans a second expanded list on
the same `write_out` call stack. Section 1334 reports the inclusive allocator
coordinates. The root is therefore a missing observation of command-scanner
ownership while these lists coexist, not persistent semantic reachability.

The generic fix counts command-owned transient input and macro-argument
buffers once and records the complete `write_out` one-word high-water before
the artificial stopper is consumed. Stored token-list replays and parameter
ranges share existing ownership and are deliberately excluded. A focused
positive control proves that 600 scanner-owned token words increase the
one-word arena. Its negative control proves that merely interning an unbound
immutable 600-token host list does not create a TeX allocation owner. A second
control proves that macro-argument ranges and stored replays are not double
charged beside genuinely owned argument and recovery buffers.

Exact compatibility TRIP remains green. Official e-TRIP's memory row advances
from `2999` to the exact `3317` words with exact normalized DVI and zero
projected semantic divergences. The independent next front is the final
multiletter control-sequence count, `409` expected versus `408` actual, and is
recorded observed-only as `umber2-jg04.52`.
