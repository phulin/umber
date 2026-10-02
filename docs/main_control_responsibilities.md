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
semantic evidence only when requested. `main_control/host_boundaries.rs`
handles demanded diagnostic/error publication and eligible outer-paragraph
checkpoints. Scanner consumers reclaim their own scratch; no successful-command
settlement or cleanup stage exists. Failure latches the session; replay restores every aggregate
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

The native fetch/reswitch loop retains its admitted context across RELAX,
ordinary group, integer/dimension register, and arithmetic commands. Each ends
the actual scanner loan before applying its source-free typed semantics.
Register and arithmetic operands are scanned in the already-live processor;
they construct no unused full command machine. Group close uses the sole
processor's native unsave and synchronous restoration callback, with
`aftergroup` entering final input at that same point. Real diagnostic,
recoverable, observer, tracked, pending output, page-owner succession,
alignment delivery, synchronous World effects, `afterassignment`, and caller-budget demands release admission for the common
ordered settlement tail. Continued commands settle actual scratch and update
save-stack high-water accounting individually. Driver operation counts include
every completed native command, including those before a later cold call or
error; no command receives a new or separate fuel policy.

The End/Dump arm alone reads the current page, contribution list, and dead-cycle
termination condition. After delivering the stop command, it admits
the real input owners for final cleanup or backs up that same stop before
ejecting residual material. No ordinary command constructs a contribution
view to anticipate a stop that it did not receive.

Deferred output observations have a demanded owner. Only an observed output
routine creates its buffer; settlement consumes that buffer in the established
shipout/effect order and checks the same evidence bounds. Unobserved ordinary
commands own no empty page-output receipt and perform no reset of one.

The persistent interpreter owns canonical command state. Its processor method
returns the real `CommandProcessor` directly; Rust's mutable borrow enforces
exclusive access. No lifecycle facade or per-command entry/completion/live
counters duplicate that proof. The processor's actual drop still unwinds
scanner scratch and ends delivery mode. Native prefix, ignore-space, font-bank,
and paragraph-start selection read the delivered packed key, reconstructing a
meaning only when a genuine operand or diagnostic consumes it. These reads
use the current command after each fetch or reswitch and store no second
meaning column.
