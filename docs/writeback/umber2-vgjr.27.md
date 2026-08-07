# umber2-vgjr.27 -- typesetting allocation repair

## Ownership repair

`ParagraphTape` now stores each analyzed breakpoint and its trace projection in
one `BreakSite`. This removes the separately allocated, positionally indexed
trace vector while preserving the paired semantic/physical `NodeSequence`,
wide prefix metrics, and materialization actions.

The sole detached math transaction now reuses conversion-local storage across
the postorder sub-mlist schedule. Expanded-choice nodes, style markers, the
explicit choice stack, Appendix G work items, and final output staging remain
one transaction-private scratch owner. Dependency discovery similarly reuses
its view, request vector, and uniqueness set. Empty observation subtrees are
represented by the existing empty `Replay` value without first allocating an
empty sequence wrapper. No source topology, converted layout, pack observation,
or diagnostic event is omitted.

## Bounded profile and measurements

A Valgrind DHAT reduction at structural depth 100 attributed the former
per-list allocations to fresh choice/dependency buffers, first/second-pass
vectors, and replay leaf packaging. With storage reuse and the empty-replay
fast path, that probe fell from 1,717 allocations and 234,164 requested bytes
to 820 allocations and 101,584 bytes.

The unchanged release allocation ceilings pass under `MemoryMax=512M`:

| Row                        | Allocations | Requested bytes | Ceiling              |
| -------------------------- | ----------: | --------------: | -------------------- |
| `linebreak_long_paragraph` |          12 |     384,619,648 | 12 / 19,000,000,000  |
| `math_deep_submlist_stack` |     160,044 |      23,855,088 | 180,000 / 42,000,000 |

The benchmark source and Program 10 accounting are unchanged. The repair adds
one composite breakpoint record and transaction-private reusable capacity; it
does not add another paragraph, math-layout, metric, observation, or
publication authority.
