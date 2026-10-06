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
