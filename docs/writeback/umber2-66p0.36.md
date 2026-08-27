# `umber2-66p0.36`: remaining command-core architecture audit

## Evidence authority and exclusions

This read-only audit uses the newest exact whole-engine profile available on
integrated main: the `umber2-66p0.35` candidate capture from 2026-08-27
08:47:15 UTC. Its force-frame-pointer profiling ELF is 387,550,416 bytes,
SHA-256
`e53b3cd05d83d54bcd7a3d8b518df1c0310a9f9c70d4126647259193b9974a94`,
and build ID `2ae4af754f4d94d57fd0bf4ed9f23b70278530b3`. The 199 Hz `cycles:u`
capture has 1,484 samples and 17,506,620,130 period-weighted cycles, zero lost
samples, and no symbolization errors.

The authenticated offline workload is arXiv `2606.12566`, selected
`ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format object `ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and distribution root `721e833071d92bba` with manifest SHA-256
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`.
It stopped intentionally at status 1 with exact work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`.

The capture predates the concurrently integrated singular-fuel patch, so its
`ProcessorFuel::charge` samples are excluded as completed by
`umber2-66p0.34`. Also excluded are the completed input-ancestry tracker
(`umber2-66p0.32`), ErrorStop mailbox (`umber2-66p0.35`), destination and
scalar handoffs (`umber2-7asg.10` and `umber2-7asg.11`), processor capability
copies (`umber2-7asg.12`), and mode journal handoff (`umber2-7asg.13`). Command
resolution is excluded because `umber2-66p0.30` rejected that whole-engine
candidate. Macro-match scratch is reserved by active `umber2-66p0.33`.

The following cycle sets were partitioned directly from the raw stack stream.
Error-context ancestry has no overlap with either other owner. File-framing
ancestry receives its complete set first; the command-context row then excludes
the seven framing samples containing both symbols. Thus the three rows total
843,766,111 cycles (4.8197%) without counting any sample twice.

## Candidate 1: project only displayed error-context levels

`InputState::render_context_for_levels` owns 36 samples and 439,577,931
ancestry cycles (2.5109%). It first pseudoprints every eligible input level into
an owned `Vec<ErrorContextLevel>`. Only afterward does
`tex_state::print::render_error_context` apply `\errorcontextlines`, keeping
the current level, a bounded prefix, and the bottom level while discarding the
other already-built strings. The measured 20M endpoint makes this duplicated
projection visible because fuel exhaustion renders a deep live input stack.

The simpler end state combines selection with the existing innermost-to-bottom
walk. It pseudoprints the current and budgeted levels, retains only the one
possible bottom replacement, emits the ellipsis once, and renders those values
through the shared line-width kernel. This deletes the eager all-level DTO
materialization and the second full-level traversal; it adds no cache,
persistent owner, special execution path, or heap indirection. Exact risks are
diagnostic text identity for negative and zero `\errorcontextlines`, omitted
exhausted token lists, scantokens traversal, the first real-file stop, and
current/bottom aliasing. Expected implementation risk is medium because the
change is confined to diagnostic projection but its output is byte-exact.

## Candidate 2: admit one command context per ordinary operation

After assigning every file-framing-overlap sample to candidate 3,
`Universe::command_context` owns 22 samples and 270,070,463 ancestry cycles
(1.5427%). The immediate callers are `prepare_operation` (159,915,184
cycles), `apply_prepared_operation` (49,089,134), `record_save_stack_usage`
(24,657,949), `fire_pending_page_output` (24,606,762), and
`local_glue_pointer_reassigned` (11,801,434). Current main reconstructs the
same borrow-only facade repeatedly inside one operation: tracked-region
probing, named-input publication, capability refresh, command delivery,
semantic apply, and accounting reopen the same admitted generation even when
no executor barrier intervenes.

The simpler end state makes the existing operation-local `CommandContext` the
one admission owner and passes mutable or shared reborrows through those
phases. It must end before suspension, rollback, resource acquisition, or any
other executor barrier, exactly as the current lifetime contract requires.
This consolidates facade construction and generation admission; it does not
retain a context in `MainControl`, add lifetime machinery, or duplicate
Universe state. Expected risk is high: the semantic boundary is unchanged,
but borrow splitting across command, mode, page, World, and observation work
will expose any phase that genuinely requires a fresh Universe admission.

## Candidate 3: delete the persistent file-framing event queue

`MainControl::drain_file_framing_events` owns 11 samples and 134,117,717
ancestry cycles (0.7661%), including 47,894,343 direct self cycles. Seven of
those samples contain 86,223,374 `Universe::command_context` cycles and belong
only to this candidate in the partition. `CommandStateRoots` currently owns a
snapshot-visible `Vec<FileFramingEvent>`; source push and retirement enqueue
`Open` or `Close`, processor sites repeatedly drain it, and five main-control
step seams poll and drain the residue. This turns two immediate transcript
effects into persistent rollback state and repeated empty-queue work.

The simpler end state returns the source transition to the caller that already
owns the live command context and renders `(name` or `)` at that exact semantic
point. The persistent vector, snapshot count, raw take API, and executor drain
polls then disappear. Any stack leaf that cannot print directly returns at
most its one call-local transition through its existing result, rather than
creating another queue or owner. Expected risk is medium-high: exact open/close
ordering relative to outer-validity diagnostics, root startup, traced
scantokens, rollback, resource suspension, and final cleanup must remain
unchanged.

## Recommendation

Candidate 1 should run first. It has the largest disjoint measured ceiling,
deletes the clearest produce-everything-then-discard pipeline, and does not
need a wider mutable borrow or alter ordinary command delivery. Candidate 3 is
the next cleanest ownership deletion but crosses more ordering sites.
Candidate 2 has a meaningful measured ceiling and the desired single-owner end
state, but should wait until an implementation sketch proves that one ordinary
operation borrow suffices without new lifetime structure.

The three implementation tasks are tracked, respectively, as
`umber2-66p0.38`, `umber2-66p0.39`, and `umber2-66p0.40`. They are linked to
this audit and the `umber2-66p0` epic. The concurrent `umber2-66p0.37` audit
confirmed its command/preflight, diagnostic-vector, and `\ifx` ownership
shortlist does not overlap these three owners.

No production code was edited and no build, benchmark, or test command was run,
as required by the read-only audit.
