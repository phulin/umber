# Main-control responsibility boundaries

`MainControl` remains the one executor and owns command, mode, observation,
box, alignment, output, and settlement state. Private modules group methods
by their actual semantic responsibility; they add no durable state or second
command front. [Main-control dispatch](main_control_dispatch.md) maps the
packed switch, synchronous typed scanners, native publication, and semantic
routines.

`main_control/resources.rs` resolves typed font, input-stream, and PDF-image
requests through the host provider, installs answers in the existing
capabilities, and validates the exact resource. A declined request unwinds the
call for full-checkpoint replay. No scanner cursor or operation payload
crosses that boundary.

`main_control/root_source.rs` registers already-supplied immutable source
bytes and owns the one-root rule, editor rebind, startup string accounting,
and root completion policy. `main_control/discretionary.rs` applies the live
`\discretionary` and `\-` groups and parts on the same mode/node owners.
Their operand scanners belong to `tex-command`.

`main_control/semantic_owners.rs` lends real owners and publishes ordered
semantic evidence. `main_control/settlement.rs` closes actual scanner scratch,
handles diagnostic/error completion, and seals eligible outer-paragraph
checkpoints. Failure latches the session; replay restores every aggregate
root rather than rolling back one command.

Ordinary dispatch must not reacquire generation ownership merely to inspect
page-builder scheduling flags or native save-stack accounting. Those scalar
queries belong to their existing Universe-owned page and dense-state stores;
they lend no token, definition or node payload. The main-control prologue admits
its command context only once. Scanner and semantic consumers still acquire
their actual mutable generation loan, and host transitions retire it before
World publication. Output settlement reads the current page flags once at its
own scheduling seam; it does not duplicate the same capture and fire-up in an
outer wrapper. Named-list tracing admits a context only when a real queued
push needs publication. These demand boundaries preserve diagnostics, page
continuation, aftergroup and output order while removing ordinary-command
owner admission and empty publication work.

The source-free box, alignment, paragraph, math, and shipout kernels keep their
existing authority boundaries. Main control calls them with native typed
operands after scanning. Observed and unobserved execution share all semantic
arms; tests compare terminal/error behavior, ordered observations, artifacts,
resource replay, and canonical oracle output.
