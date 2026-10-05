# Active input-frame ownership

The native reader currently finds its live cursor by loading the input-row
vector, checking its length, multiplying its last index by the complete row
size, and selecting the row's storage variant. Every terminal callback can
change input, so this selection repeats for every ordinary macro word. The
2026-10-04 dense-relax profile attributes 88.7% of samples to the shared reader
and interpreter; its frame cursor/limit access and storage/token dispatch are
prominent. Compact meanings remove row reconstruction, but do not remove this
input ownership cost.

The selected replacement follows TeX's `cur_input` and `input_stack`: one actual
current input level belongs directly to `InputStack`, while a vector owns only
suspended levels. The current level retains its complete frame, source or token
storage owner, immutable macro body and final argument-region coordinate. There
is no duplicate cursor, cached top pointer, native eligibility front or second
reader. Ordinary input reads borrow the current level directly.

## Transitions and ownership

An empty stack has no current level and no suspended levels. Pushing suspends
the actual previous current level and installs the newly admitted level.
Popping releases the retiring level's actual insertion suffix or final argument
region while its payload remains live, then restores the previous current level
from the suspended vector. Source retirement keeps the same projection ordering
and occupied-buffer bookkeeping. Macro arguments still retire above their
owning body; only the body's retirement returns its final argument region.

Macro-frame construction folds into its actual activation caller.
Parameterless calls need no separate argument-publication call frame. Input
conservation and draining genuinely depleted owners remain the existing
transitions.

The final traced-word vector, argument arena, live parameter count, source-slot
owners, input-level identities and cold context epoch remain the existing
owners. This change adds no allocator, arena, reference acquisition, retained
history or new accounting scalar. Native capacity and high-water accounting
use the actual suspended count plus current-level presence. Retained exhausted
v-templates, recovery insertions, aftergroup linking and replay completion use
these same transitions.

The current owner moves on real input transitions. Avoid reconstructing an
input level solely to consume or retire it. Moving the current frame can cost
more than appending one old vector row, so measure short nonempty macros as
well as dense replacement bodies. Fix any regression at the actual transition
rather than adding a compatibility or alternate execution path.

Exhaustion selects its real retirement once. An ordinary popped owner returns
only a genuine replay completion when present; a retained terminal or template
returns its cold boundary with the existing identity. Do not retire through
an ambiguous optional result and then inspect the new current owner merely to
discover whether a pop occurred. Conservation uses this same retirement.

The shared reader retires exhausted resident input at its inline boundary,
after the word-span borrow ends. The actual storage owner's pop and release
remains out of the reader's frame. Source acquisition and parameter substitution
enter one cold handler; retained terminal/template input and replay completion
keep their existing demands. No ordinary exhaustion passes through the cold
source/parameter dispatcher.

## Cold traversal and capture

The semantic level order is suspended levels from oldest to newest followed by
the current level. Diagnostics, origin discovery, resource contexts, summary
hashes and source-coordinate validation traverse those actual owners. They may
use a borrowed iterator; they must not collect a new vector merely to recover
the former slice representation. The old row-vector field, slice dereference
and generic slice-index adapter are deleted. Indexed access is retained only
where an actual validated semantic coordinate requires it; the ordinary reader
never computes a top index to access its current owner.

Input capture keeps its existing admission rules. Quiescent captures contain
no macro bodies, argument borrowers or active argument regions. Copying a
capture preserves the suspended/current distinction and their source owners;
restoring discards abandoned owners, installs the captured levels, clears the
actual argument arena and advances the cold context epoch. Capture, rollback,
resource replay and branch validation gain no extra stage or checkpoint cache.

## Validation and measurement

Run the command, execution, state and incremental suites, the complete native
suite, all authoritative quality gates and exhaustive canonical comparison.
Canonical debt must retain its exact ordered recurrence map; do not replace the
independent TeX oracle with an old Umber executable.

Compare frozen binaries on the same INITEX workloads and CPU: relax, empty and
short nonempty macros, dense replacement bodies, groups, assignments and boxes.
The preceding compact-meaning reader uses 196 marginal instructions per relax,
848 per two-relax macro body, 200 per empty macro and 1,222 per empty group.
Repeated cycle measurements and reader profiles explain the result. Completing
the owner replacement and resolving regressions remain the objective; no
numerical gate substitutes for either.
