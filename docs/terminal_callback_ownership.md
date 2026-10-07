# Borrowed full-access terminals

Status: implemented for the shared command reader; validation is in progress.

## Code and ownership boundary

Resident consumers execute while the sole reader retains its actual input
slice and cursor. Their concrete consumer types carry the real grammar and
static admission capabilities: digits, keywords, characters, and quiet native
commands. These consumers remain statically compiled.

A full-access terminal releases the input borrow before its callback receives
the actual processor. Such a callback can expand, substitute, scan another
operand, modify input, or execute an operation. Its closure type does not need
to specialize the entire reader. The existing `terminal_consumer` entry borrows
that closure as a `dyn FnMut` with the native terminal signature. It returns
the borrow itself. There is no callback wrapper, allocation, consumer tag,
command product, new reader, or retained continuation.

All full-access callbacks with the same processor type use that same reference
type. Scanner and main-control fetch roles keep their existing reader entry
and alignment behavior. The reader calls the actual callback indirectly only
at full-access terminal delivery. Its immutable word/meaning loans end when
the callback returns. Native interpreter and resident grammar consumers do
not use this erased callback entry.

Raw operand callbacks remain concrete. Unlike expanded terminals, many raw
callers immediately return a single spelling or compare one compulsory token;
their callback and return branch must remain available for caller simplification.
The raw reader delivers a meaning loan only when requested or when the actual
exceptional delivery already owns one. Ordinary spelling-only reads outside an
active scanner do not resolve a meaning just to test outerness: no outer
restriction applies there. Matching and other active scanners still validate
live outer meanings. Alignment, suppression, recovery and observation resolve
their demanded meaning at the actual exceptional delivery boundary.

This is the same synchronous raw reader and the same control-sequence creation
scope. No retained callback, meaning cache, fast-path owner or completion stage
is introduced.

## Resident traversal placement

A short parameterless macro retains its actual immutable body and installs one
input row. Its two terminal words need no full-processor callback. The dense
activation measurement, with identical terminal streams, nevertheless finds
about 634 native instructions per child activation against 216 in the pinned
pdfTeX oracle. Sampling places most work in the enclosing main-control reader
and its separately outlined resident span call. The reader constructs a
closure borrowing its disjoint owners on every span entry, spills those
references for the call, then decodes the returned boundary before continuing.

The physical word traversal belongs directly inside the selected storage loan.
Remove the separately outlined `consume_span` function and its closure adapter.
One local traversal body admits words through the existing concrete consumer;
packed and traced storage use their actual projections of that same body.
Ordinary acceptance advances the final frame and charges the next word in
place. A parameter marker or exhaustion returns to the existing input
transition; a declined word releases the same lexical loan before the
full-access callback. Macro activation, scanning and execution still occur
only after that borrow ends.

The admitted slice already ends at the frame limit. Its iterator's remaining
suffix therefore supplies the exact `frame_live` fact after each word, with
no second frame-limit read. Fuel is borrowed once with the other disjoint
owners. Each accepted word still advances the actual frame before admission;
no copied cursor or deferred position write is introduced.

This deletes a call boundary and its temporary reference packet. It creates no
second reader, retained cursor, macro frame, callback owner or completion
phase. The word iterator and admission scalars have lexical scope inside the
loan. Check generated stack size and recursive expansion tests as well as
instruction and cycle measurements: compiler spills can survive removal of
the source-level function, and larger enclosing code can lose performance.

## Nested scans and checkpoints

The callback borrow belongs to its synchronous scanner call. Nested scans
borrow their own local callbacks while the parent call waits on the stack.
No callback pointer, captured local, or trait-object reference enters command
state, execution state, input frames, formats or checkpoints.

On a resource miss or error, ordinary Rust unwinding releases the callback
borrow. The existing full checkpoint restores the actual input, group,
execution and effect owners. The new entry adds no successful completion
cleanup or recovery protocol.

The by-value callback entry is removed. Callers lend their actual closure;
there is no compatibility constructor or alternative parser.

## Verification

Check that compiled full-access readers share the callback-reference type
rather than individual closure types. Preserve actual word deliveries,
meaning selection, fuel, observations, diagnostics, alignment and group
ownership. Compare semantic and replay tests against the preceding code,
including nested scans and exceptional delivery. Measure frozen instruction
and cycle workloads against both the preceding box-prefix implementation and
the pinned pdfTeX oracle. A smaller executable alone does not establish a
speedup.
