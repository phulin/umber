# Tracked-region semantic read coverage

Status: implemented authoritative coverage contract. The generic state-layer
recorder, command-side routing, execution-side routing, cross-layer
perturbation harness, and exact-World unrelated-change proof are complete.

This document defines the only region for which the first generic dependency
coverage proof is made. It is a recording contract, not permission to replay
the region. The detached record has no command continuation, mode transition,
page transition, effect slice, artifact, or paragraph output.

## Supported region

The supported region is one _ordinary outer main-control operation_: one call
to `MainControl::advance`, internally owned by the non-nested `Advance`
transaction in `MainControl::execute_operation`.

The region begins after that operation's aggregate `StepSnapshot` has been
captured and immediately before `apply_operation`. It ends after command
delivery, expansion and scanning, executor application, nested command
episodes, diagnostics, page/output tails, and receipt admission have all
completed, but before the aggregate savepoint commits. `MainControl` is the
begin/finish owner because it is the lowest layer that surrounds both
`tex-command` and `tex-exec`; the command crate cannot depend on the executor.

The following are part of the one parent region and must not begin nested
regions:

- prefix and scanner episodes;
- executor-requested redispatch and `do_assignments` episodes;
- nested output-routine, math, discretionary, box, and page-building work; and
- every command processor borrow constructed while applying the operation.

The startup filename scanner, diagnostic expansion API, explicit legacy
`alignment_step`, `JobStart`, `FinishEnd`, `Finalize`, named checkpoint capture,
and host-side resource registration are outside this first region. A later
contract may add them only as separately owned region kinds. A canonical runner
step is too large: its main-control phase may batch up to 256 ordinary
operations. A command delivery or scanner episode is too small: it ends before
stomach and page/output reads are known.

### Completion and failure

On ordinary `Progress`, the owner finishes the tracked region before committing
the savepoint, then publishes a successful detached record only after the TeX
operation commits. Record publication is a call-local optional sink operation
and cannot make an already valid TeX operation fail.

On resource suspension or a rolled-back error, the owner abandons the region
before restoring the aggregate savepoint. A fatal or other partial-commit path
marks the region unsupported and publishes no record. A failure to finish a
record is optimization failure only: the original operation keeps its existing
commit, diagnostic, effect, and error behavior.

Nested begin is an error. Nested work records into the outer operation. The
existing journal-lineage rejection remains authoritative: group exit,
checkpoint capture, or rollback after the mark makes the attempted record
unsupported. Entering a group is not by itself a barrier, and assignments may
be recorded inside a group when the operation finishes before that group exits.

## Fail-closed mechanism

`Universe` owns one typed poison operation beside begin, finish, and abandon.
Later implementation exposes it to `tex-command` only through
`CommandContext`; execution calls it through the aggregate `Universe` facade.
It has these semantics:

- with no active recorder it is an allocation-free semantic no-op;
- with an active recorder it stores the first typed barrier reason;
- repeated barriers are idempotent and cannot restore eligibility;
- finish of a poisoned region clears all partial observations and returns a
  typed unsupported-region result, never a partial record; and
- rollback explicitly abandons before aggregate restoration, so restored
  recorder state or observations cannot leak into a retry.

Barrier reasons are a closed typed family, not strings. They distinguish at
least unsupported command state, unsupported execution state, unsupported
World/resource fact, irreversible effect/materialization, unsupported host
capability, fatal partial commit, and environment-timeline change. The reason
is diagnostic evidence for tests and development; consumers may rely only on
the binary reusable versus unsupported result.

No read may silently omit an observation because its semantic projection is
unavailable. Such a getter must either gain the matrix's typed projection at
its aggregate access boundary or poison the region before returning the value.

## Exhaustive coverage matrix

“Exact” means the narrow key shown. “Aggregate” means one versioned canonical
projection of the named root. A dependency value must be allocation-independent
and detached; a live `Symbol`, `FontId`, token-list id, glue id, node id, source
id, path, or pointer is never a recorded value.

| Semantic family                 | Facts read in the region                                                                                                                                                                                 | Key and value                                                                                                                      | Read owner                                                                    | Disposition                                                                                                                                         |
| ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| Meanings                        | Current control-sequence and active-character meaning, including macro identity                                                                                                                          | `Cell(Meaning, symbol)` and canonical meaning projection                                                                           | command                                                                       | Exact; every lookup routes through `CommandContext::meaning`                                                                                        |
| Environment values              | Count, dimension, skip, muskip, token and box registers; integer, dimension, glue and token parameters                                                                                                   | Scope-free `Cell(CellId)` and typed scalar/content projection                                                                      | command for scanners; execution for application/layout                        | Exact; environment writes remain journal-derived and are not observations                                                                           |
| Font selectors                  | Current font and the 48 math-family selectors                                                                                                                                                            | `Cell(CurrentFont, 0)` or `Cell(MathFamilyFont, index)` and canonical font projection                                              | command and execution                                                         | Exact                                                                                                                                               |
| Code tables                     | Catcode, lc, uc, sf, math, and delimiter code for one scalar                                                                                                                                             | `Code { table, scalar }` and integer                                                                                               | command for tokenization/scanning; execution for layout/math                  | Exact; all-table traversal uses `CodeGeneration(table)` and a canonical table projection                                                            |
| Font identity and metrics       | Identifier, name, character/ligature/kern/math metrics, parameter count or value, hyphen/skew character, pdfTeX code and shaping state                                                                   | `Font { field, font, index }`; scalar or canonical font/field projection                                                           | command for font enquiries; execution for shaping, math, packing, diagnostics | Exact where a field exists; an unprojectable live font fact is a barrier                                                                            |
| Hyphenation                     | Patterns, exceptions, and saved hyphenation codes for the selected language                                                                                                                              | `HyphenationPatterns`, `HyphenationExceptions`, or `HyphenationCodes`; per-language projection                                     | command while loading/enquiring; execution while breaking                     | Aggregate per language                                                                                                                              |
| Immutable input                 | Registered content, normalized physical line plus terminator, and source-registration outcome                                                                                                            | `InputRecord(hash)` and `PhysicalLine { content, terminator }`; content/projection value                                           | command                                                                       | Exact immutable content; provenance and runtime source ids are excluded from identity                                                               |
| Command input state             | Current normalized line, source/token-list cursor stack, replay levels, macro arguments, alignment interception, scanner/conditional delivery state                                                      | `InputLine` or `InputStack`; versioned command-owned semantic projection                                                           | command                                                                       | Aggregate; any field omitted from that projection is an unsupported-command-state barrier                                                           |
| Input streams and terminal      | Open/EOF state, selected record and cursor, terminal cursor and supplied line                                                                                                                            | `InputStream(slot)` or `World { InputStream/TerminalInputCursor, index }`; scalar/content projection                               | command through `CommandContext`                                              | Exact by slot/cursor; interactive recovery not represented in `World` is a host-capability barrier                                                  |
| Resource lookup                 | Required/probe outcome and immutable supplied bytes for input, font, or external image requests; loaded-resource selection                                                                               | `World { InputResource/LoadedResources, stable request identity }` plus `InputRecord`/font/image identity                          | command for input/probes; execution for fonts/images                          | Exact request identity or bounded aggregate. A `Need` abandons on suspension; host resolver policy, URL, cache, or response timing is excluded      |
| Execution modes                 | Current mode, inner-mode result, mode depth, list metadata needed by an enquiry, and `lastnodetype`                                                                                                      | `Engine(Mode/InnerMode/LastNodeType)` and versioned executor-owned projection                                                      | execution records before lending the fact to command                          | Exact scalar where possible, otherwise aggregate; unprojected list continuation state is a barrier                                                  |
| Groups                          | Execution group depth and innermost group kind                                                                                                                                                           | `Engine(GroupLevel/GroupType)` and scalar                                                                                          | execution through `Universe`                                                  | Exact. Group exit also triggers the independent journal-lineage barrier                                                                             |
| Conditions                      | Conditional depth, type, selected branch, and complete stack when a loop or diagnostic examines more than one frame                                                                                      | `Engine(ConditionLevel/ConditionType/ConditionBranch/ConditionStack)` and command-owned scalar/projection                          | command                                                                       | Exact top fact or aggregate stack; condition mutation is command transition state, not an environment write                                         |
| Paragraph inputs                | `parshape`, e-TeX penalty arrays, language parameters, layout parameters, fonts, hyphenation, and source list                                                                                            | Existing exact cell/font/hyphenation keys plus `Engine(ParShape/PenaltyArrays)`                                                    | execution                                                                     | Supported reads are explicit; completed paragraph output and continuation are not added to this record                                              |
| Page scalars                    | Page dimensions, integers, marks, and mark classes                                                                                                                                                       | `PageDimension`, `PageInteger`, `PageMark`, or `PageMarkClass`; scalar/content projection                                          | command enquiries and execution                                               | Exact                                                                                                                                               |
| Page roots                      | Contents, contributions, current page, insertions, discards, split discards, break state, and pending fire-up                                                                                            | `Page(DependencyPageField)` and canonical root projection                                                                          | execution                                                                     | Aggregate by listed root; a page getter lacking one of these projections is a barrier                                                               |
| Clock, timer, and random        | Fixed job clock, pdfTeX elapsed timer, RNG state/result, and seed                                                                                                                                        | `World { JobClock/Rng, 0 }` or `Engine(PdfTimer/PdfRandom)` and scalar/state projection                                            | execution                                                                     | Supported only through virtual `World`; wall clock or OS entropy is a host-capability barrier                                                       |
| Policies and streams            | Interaction mode, shell-escape/effect policy, output-stream open/target state, and materialization status                                                                                                | `Engine(InteractionMode/PdfShellEscape)`, or `World { EffectPolicy/ShellEscapePolicy/OutputStream/MaterializationBarrier, index }` | command for interaction; execution for effects                                | Observe virtual policy and stream facts; real publication/materialization is a barrier                                                              |
| PDF state and external objects  | Object, position, form, page and external-image ledgers or enquiry facts                                                                                                                                 | `Engine(PdfObjects/PdfPositions/PdfForms/PdfPages/PdfExternalImages)` and canonical aggregate projection                           | execution                                                                     | Aggregate by ledger; host image bytes also use the resource row                                                                                     |
| Nested pure queries             | Result of a separately bounded pure memo whose own dependencies are complete                                                                                                                             | `Query { domain, identity }` and canonical result projection                                                                       | execution                                                                     | Allowed only as the documented bounded parent dependency; it never starts a nested tracked region                                                   |
| Diagnostics and virtual effects | Parameters/state read to choose or render diagnostics, deferred writes, specials, virtual stream writes, and staged artifacts                                                                            | Their underlying exact dependencies; produced records are outputs, not read dependencies                                           | command and execution                                                         | Virtual outputs may occur, but this record does not capture them for replay. Any read used only by rendering is still semantic and must be observed |
| Irreversible effects            | Effect commit to a real host, file publication, shell execution, or externally visible artifact materialization                                                                                          | No reusable key                                                                                                                    | execution                                                                     | Barrier before publication. The TeX effect still follows existing commit semantics                                                                  |
| Unvirtualized/unsupported facts | Host callbacks, cancellation result, resolver/cache policy, filesystem metadata not admitted to `World`, opaque foreign state, pointer/allocation identity, or an aggregate with no canonical projection | No reusable key                                                                                                                    | layer that first reads it                                                     | Barrier before the read can influence semantics                                                                                                     |
| Operational-only facts          | Fuel burned/limit, cancellation polling before mutation, telemetry, profiling clocks/counters, capacities, cache hits, and observer presence                                                             | None                                                                                                                               | runner or owning layer                                                        | Excluded only while they cannot change committed TeX state, diagnostics, effects, artifacts, or checkpoint schedule; otherwise barrier              |

This matrix is exhaustive by rule: adding a semantic getter or a variant to
`DependencyKey`, `DependencyEngineField`, `DependencyPageField`, or
`DependencyWorldField` requires placing it in one row and adding the matching
proof. “Diagnostic-only” is not an exemption because transcript and terminal
bytes are observable TeX behavior.

## Implementation ownership

`tex-state` owns key/value vocabulary, allocation-independent projections,
changed-at mutation wiring, recorder poison/finish behavior, and the restricted
`CommandContext` facade. It owns no command or execution policy.

`umber2-trcn.4.2` owns the command half: meanings and scanner environment
reads, code tables, command-side font and page enquiries, immutable input and
input-stack projections, conditions, stream/resource facts, and barriers from
command diagnostics or unsupported host capabilities. It must use
`CommandContext`; it must not expose substores or create a second input
recorder.

The implemented command admission observes `InputLine`, `InputStack`, and the
four conditional projections once when a processor borrow enters an active
outer region. Its versioned input projection replaces source, input-level,
token-list, symbol, and provenance handles with immutable bytes, stack order,
semantic tokens, and control-sequence spellings. A continuation whose
stack-relative macro/alignment identity or pending resource request has no
canonical translation is an unsupported-command-state barrier before it is
read. Existing borrow-scoped host file lookup and file-probe capabilities are
unsupported-host-capability barriers; moving those outcomes into the virtual
`World` vocabulary is required before such an operation can publish a record.

The implemented execution admission begins after the ordinary operation's
savepoint, records the canonical mode-nest projection before lending its mode
and list facts to command processing, and finishes before savepoint commit.
Nested command work contributes to that same recorder. `Universe` getters
observe execution-side environment, font, hyphenation/layout, page, PDF, and
virtual `World` facts through an atomic inactive fast path. PDF and page root
projections currently share one conservative changed-at clock per family.
World projections are field-specific: stream slots and input-resource request
identities are exact, loaded resources retain the matrix's bounded aggregate,
and the other policy, clock, RNG, cursor, and materialization facts have
independent canonical values. The retained driver-only `world_mut` capability
compares only previously tracked World-backed facts around its mutable borrow
and advances only the stamps whose values changed. Capability-specific
aggregate paths keep their direct exact stamps. No World mutation globally
invalidates unrelated environment, page, command, or engine facts.
Suspension and rollback abandon before restore; timeline changes, unsupported
host resource selection, fatal or partial commit, and irreversible effect
materialization fail closed. A committed attempt is returned only after the
ordinary TeX operation commits, and recording failure never changes that
operation's result.

`umber2-trcn.4.4` owns the cross-layer proof and the final source audit. Neither
implementation child may weaken a matrix row locally; an unimplemented row is
a barrier until coverage and proof exist.

## Required perturbation proof

The closure harness uses one representative ordinary operation for every
matrix disposition and makes three assertions where applicable:

1. changing the exact fact read makes validation reject the detached record;
2. changing a nearby unrelated fact leaves validation green; and
3. taking a barrier, rollback, suspension, fatal, nested-begin, or unsupported
   timeline path publishes no record and leaves no recorder active.

The harness must include fault injection that suppresses one supported read
observation while leaving the semantic read itself active. It then perturbs
that fact and must detect the false-green validation. A test that merely
enumerates keys or getters is insufficient.

Finally, paired runs with recording disabled and enabled must have identical
committed TeX state, diagnostics, effect records, artifacts/DVI/PDF, resource
requests, errors, operation termination, and named-checkpoint schedule.
Disabled execution calls the existing ordinary path: it does not begin a
region, allocate a recorder, compute command/mode/page aggregate projections,
or execute barrier policy beyond the existing predictable inactive checks.

## Executable closure evidence

The source audit is compile-closed in
`tex_state::dependency::tests::every_documented_key_variant_is_classified_invalidated_and_backdated`.
It exhaustively matches every nested code-table, font, engine, page, and World
field without a wildcard, then gives every key variant an exact red
perturbation and an unrelated green perturbation. Adding vocabulary without a
matrix disposition therefore fails compilation; omitting a listed key from
the 101-entry perturbation inventory fails the test.

`tex_exec::main_control::direct_tests::tracked_region_coverage` supplies the
cross-layer proof. Its command case reads a count register through real
conditional scanning, proves a nearby register remains green, proves the read
register turns red, and removes that observation from the detached record as
deliberate fault injection. The harness detects the resulting false green.
Paired complete jobs and paired resource suspensions compare termination,
aggregate state, diagnostics and virtual effects, committed artifacts,
prepared DVI pages, resource requests, and named-boundary schedules with
recording disabled and enabled.

`tex_state::dependency::tests::every_documented_barrier_discards_partial_evidence_and_resets_the_recorder`
exercises every closed barrier reason and proves its partial observations are
not reusable. The cross-layer suite proves the nested-begin and fatal paths;
`tracked_advance_abandons_before_resource_suspension_rollback` and
`tracked_group_exit_fails_closed_at_the_journal_timeline_barrier` prove
suspension abandonment and group-exit failure. State-layer rollback and
explicit-abandon coverage prove each path leaves a clean replacement recorder.

### Exact World precision evidence

`tex_state::universe::tests::every_world_projection_stays_green_after_an_unrelated_world_mutation`
proves the unrelated-World mutation case for every `DependencyWorldField`.
`exact_world_keys_reject_relevant_mutations_without_cross_talk` proves exact
request and stream-slot separation, while
`scalar_and_aggregate_world_keys_reject_their_relevant_mutations` covers the
remaining field-specific values and stamps.
`world_projections_are_allocation_independent_across_universes` constructs the
same resources in different allocation orders and proves all ten projections
are equal. The matrix and supported region remain unchanged; none of this
evidence authorizes replay.

## Measurement and backend disposition

The standalone state diagnostic owns the reproducible micro-cost and retention
receipt for this substrate:

```bash
cargo run --release --manifest-path benchmarks/tex-state/Cargo.toml \
  --bin dependency_gate
```

It uses median optimized timings, requested-byte allocation accounting, 4,096
unique facts, and a 256-cell dense environment footprint. It reports the
tracking-disabled branch against an equivalent control, active unique-read
recording, disabled and tracked mutation-receipt paths, unchanged and
backdated validation, journal-derived write-footprint extraction, rollback
backdating and temporary allocation, detached observation/write retention,
tracker retention, and the logical sizes of the public evidence types. The
`dependency_recording` filter in `state_budgets` remains the Criterion-owned
deduplicated-read cross-check.

The dated optimized numbers, toolchain identity, and frozen edit-restart
comparison are durable on Beads issue `umber2-trcn.5`; historical numbers do
not live in this architecture contract. The edit-restart run verifies the
committed workload checksums and cold DVI equality under ordinary execution,
where no driver requests a tracked region. It therefore measures the inactive
substrate and the existing named-boundary restart path, not reuse of a tracked
record.

The current `AHashMap` changed-at tracker and deterministic `BTreeMap` recorder
remain the backend. The closure measurements do not justify bank-local stamp
arrays, seen-epoch vectors, dirty sets, or another index: the inactive path is
subnanosecond at the recorder boundary, and active costs have no authorized
consumer workload against which a replacement could earn its retained-memory
and complexity cost. A downstream replay or reuse design must be separately
authorized, must retain this one-advance semantic contract, and must establish
its own end-to-end workload and latency acceptance before proposing such a
backend change. This substrate does not skip execution and makes no edit-latency
claim.
