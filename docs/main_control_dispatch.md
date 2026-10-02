# Main-control dispatch

[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md)
governs this engine. `tex-command::CurrentCommand` contains its native packed
`MeaningWord`; `control_code` projects its dispatch family without storing a
second decoded meaning. Command identity is derived at its actual diagnostic
or observer consumer from that packed word and the existing noexpand flag;
ordinary delivery stores no redundant identity column. `main_control/direct/dispatch.rs` is the one packed
mode/command switch, including TeX82 §1030's `reswitch`, §1034's horizontal
character loop, and §1211's prefix loop. Ordinary, observed, tracked,
diagnostic, nested math/accent/alignment, PDF, and resource consumers enter
these same arms.

The switch key is a demanded projection of the native word. Scalar opcode
tags occupy one byte’s range, followed by the sixteen literal catcodes,
expandable byte selectors and unexpandable sixteen-bit selectors. These
disjoint small ranges permit ordinary indexed branching; retaining the
opcode in bit 56 would create a sparse 64-bit switch for the same semantics.
The key is local to dispatch. Stored meaning words, font/definition
capabilities, formats and checkpoint records retain their native encodings.

Expanded fetching and raw horizontal lookahead retain their distinct TeX
labels. Borrowed source spans enter the actual horizontal material consumer.
Prefixes and replacement commands dispatch in place without redelivery. The reswitch label derives one local packed switch key from its actual current command, refreshing it when a prefix or replacement fetch changes that command. Scalar scan scratch belongs only to the native arms that scan scalars. Each assignment arm resolves its effective global flag once before scanning; unrelated commands read no GLOBAL_DEFS. Brace and math consumers query the live group only when that semantic case needs it. Disabled tracing prepares and reports no command evidence. Each
selected arm scans its concrete operands through the existing processor. A
source-free native arm reborrows that processor's actual context, command state,
fuel, diagnostic and observer owners while the processor remains live. The callback owns
no payload, phase, counter or separate dispatcher. The processor preserves its concrete execution provider, which lends the
actual mode owner mutably. Native application can reborrow that same owner;
subsequent expansion reads live mode facts. Ordinary groups, macro/let assignments and relax flush pending characters
through that loan and retain the existing processor. Genuine outer semantic
calls still end it where their owner requirements demand it. Context-consuming commands retain the processor and context in the same
fetch/reswitch loop. Actual consumers reclaim their scratch at consumption,
and demanded tracing records own their reporting boundary. Genuine diagnostic,
observer, output, alignment or World demands end the appropriate owner loan.
Each command consumes its real fuel and counts toward the caller's remaining
command budget; successful continuation has no settlement or cleanup stage. Native semantic
arms examine the diagnostics, pending output, observation, tracked-region, alignment delivery and synchronous World-effect
demands they actually produced; those demands end the loan before the existing
publication, checkpoint, page-owner rotation or resource boundary runs. An
error drops the real processor and context before aggregate failure settlement.
There is no precomputed eligibility classification or parallel command loop.
The same packed switch selects context-consuming semantics and genuine outer
owner calls. The delivered native token/site and command word stay in one opaque
`CurrentCommand` from raw input through expansion and that switch; spelling and
printed identity are projected only by their actual consumers. There is no
HotCommand-to-CurrentCommand repacking boundary. Native callbacks return only their actual semantic result. The shared native
continuation routine reads the live page, command, diagnostic and World owners
and updates the save-stack high-water mark in place. It copies no page-output
facts for a command that continues. Only an actual outer cut captures those
facts, before diagnostic and named-input publication; errors use that same
ordered cut. No success-fact tuple or stored continuation predicate exists. A diagnostic program cannot cross the next input acquisition:
nonempty diagnostics finish through their ordered publication boundary first.
`scan_*` and `apply_*` files group concrete routines by responsibility;
`semantic/` holds source-free semantics. The large authoritative switch stays
in one file so its mode and prefix ordering is visible together.

Genuine scanner requests parameterize their token or definition fields by
ownership domain. Token publication validates every attempt root, reserves
all native rows/words, then consumes the request while streaming final token
owners. Alignment templates use the same complete preflight discipline. There
is no duplicate rooted request, operation union, phase placeholder, readiness
mailbox, eligibility lane, prepare/apply protocol, or second executor.
Definitions and let assignments use that same retained processor. A macro
publication remains move-only through pending scan reports and any demanded
character flush, then the central assignment acquires its installed root before
releasing that publication. Let scans its complete RHS meaning before applying
any LHS change, and central assignment acquires the alias before dropping the
old cell. Protected-macro evidence uses the processor's actual observer; mutation
receipts remain demanded and publish through the common ordered tail. Pending
scan reports return the actual semantic result to the selected arm; they cannot
skip its successful afterassignment tail. Failed character flushes settle that
error before returning and never install the publication or fire afterassignment. No
complete command machine is constructed for these arms. Definitions have
counted native ownership; provisional shorthand diagnostics
consume the old owner before replacement.

`semantic_owners.rs` lends actual command, mode, box, alignment, and output
owners to a concrete call. Case-specific code publishes alignment and
assignment facts at their semantic positions. The common tail preserves show
completion, deferred print, shipout, page-output, and paragraph ordering. Native context-consuming semantics return directly without constructing
an unused command machine or entering a command settlement stage.
Observed execution emits its ordered mutation/effect/artifact receipts;
unobserved execution constructs no receipt batch. `host_boundaries.rs` admits
demanded receipts and latches failures. Actual consumers reclaim scanner scratch. It creates
no per-command savepoint or lifecycle capability.

Host misses unwind call-local scanner and semantic owners. The host restores a
full aggregate checkpoint before retry; fuel and high-water statistics remain
monotone. Genuine page, output, candidate, and aggregate owners retain their
existing cold lifetimes. Performance measurements distinguish actual hot work
from unnecessary setup and drive regression fixes; historical instruction
budgets impose no arbitrary numeric threshold.

Replay accounting counts completed driver chunks at the host boundary. A chunk
can contain several native commands; removing cold handoffs changes this
partition without changing replayed state or output. The resource-equivalence
test requires real discarded work and exact restart/response counts while
comparing complete preloaded and partitioned output byte for byte.

## Native execution owner

Main control separates the owned `CommandState`, fuel, resource capabilities,
operation observer and resource-decline slot from one owned `ExecutionState`.
That execution owner contains the existing mode, box, alignment, math,
paragraph, output, job and driver fields. This is a partition of the real
fields; it introduces no sampled facts, copied roots or additional semantic
state. Neither owner dereferences to the old flat control layout.

The reader borrows its own owners and the actual execution owner independently.
A native terminal consumer reborrows the latter with the admitted state and
command owners. The existing mode/prefix/reswitch switch remains the only
execution authority. It must perform operand scans and semantic mutation at
that consumption point, rather than returning a command product, an unbundled
command tuple, a queued operation or an eligibility handoff. Actual output,
resource, checkpoint and diagnostic demands stop the reader and return their
real result after its loans unwind.

The executor fact interface reads the execution owner's actual mode/list state.
It retains no sample across execution. Source-free routines borrow the exact
execution and command fields they need. Nested scanners use the same command
state and admitted context; neither an empty replacement interpreter nor a
replacement execution owner is installed to evade the borrow checker.

Full checkpoints continue to capture command and execution roots through the
existing aggregate contract. Group topology remains the sole native TeX save
stack, and mode roots remain the same `ModeNest`. Moving a field into its
execution owner changes no root, retention lifetime, capture eligibility or
rollback rule. Formats serialize semantic state, never either borrow interface.
