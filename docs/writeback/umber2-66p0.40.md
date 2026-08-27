# `umber2-66p0.40`: call-local file framing

## Adopted boundary

File framing is an immediate transcript effect, not future command state.
Opening a nested file or traced `\scantokens` source now returns its one name
from the input push to the processor that already owns the live
`CommandContext`; the processor renders `(name` at that transition. Source
retirement returns one `closes_file_frame` fact through its existing
`InputRetirement` result and renders `)` after e-TeX's `file_warning` but before
the next outer-validity check. Startup reads the canonical name from the
already-live root source at the driver's selector boundary.

The persistent `Vec<FileFramingEvent>`, exported event type, snapshot count,
take/render APIs, processor drains, five executor polling seams, and
incremental-driver flush are deleted. The replacement adds no queue, cache,
persistent owner, heap indirection, compaction, special execution path, or
lifetime machinery. Rollback now restores only the transcript effects and
`open_parens` that already own framing semantics; command snapshots retain no
pending framing residue.

## Evidence

Focused validation passes all 246 `tex-command` library tests, all 695
`tex-exec` library tests, and 32 EngineSession tests. These include exact
startup selector behavior, nested input suspension and retry, traced
`\scantokens` nesting, outer-validity recovery, rollback, and final cleanup.
The complete `cargo test -q --tests` routine suite passes, and
`scripts/check.sh` reports all four gates passed.

The authenticated offline arXiv `2606.12566` row stops at status 1 with the
unchanged work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`, empty standard output,
no partial PDF or input receipt, and canonical diagnostic SHA-256
`68031179ff7c37a0902ed1181ea753addeb0ea80ebc5f38881ed24fb40ac85b1`.
Per the fixed queue directive, no paired CPU profiling was run.
