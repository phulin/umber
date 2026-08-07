# Unified main-control operation authority

Issue: `umber2-vgjr.1.4`

## Authority result

Ordinary, observed, nested redispatch, and alignment delivery now enter one
`execute_operation` transaction authority. Delivery is a small typed choice;
aggregate savepoint ownership, commit and rollback, resource suspension, fatal
termination, application, publication, and telemetry no longer have parallel
implementations. `apply_operation` owns the common scan/apply/publication tail,
including main-loop parking, page-output completion, shipout publication, and
paragraph boundaries.

Observed execution occupies an optional bounded evidence slot for the whole
aggregate operation, including nested command-processor episodes. The slot is
absent and allocates nothing on ordinary execution. Both operation and deferred
page-output evidence have an append-time ceiling of 1,000,000 records and
report the existing typed resource-budget error on overflow. The active typed
execution receipt is derived from the same committed buffer; the temporary
shadow receipt harness and its inactive tests are gone.

The closeout repair makes that receipt an active consumed contract. Mutation,
resource, semantic/live-effect, artifact, diagnostic, and scalar termination
facts all enter through append-bounded methods; world/artifact deltas and
geometry evidence close before the aggregate savepoint commits. The
observer-publication seam consumes every category and verifies termination
against the published step. Independent receipt producers and category-specific
negative controls prove equality, rejection before capacity growth, and the
allocation-free absent observation slot.

The exhaustive tracer OOM was retained diagnostic evidence, not engine
nontermination. Its registry parsed Plain, Story, and Gentle simultaneously,
then replay retained a second 930,240-event Gentle stream and four full-stream
comparison side tables. Document fixtures now load one at a time, the
comparator derives keys directly, and replay compares/releases an exact prefix
while retaining the complete suffix from the first mismatch for unchanged
realignment and reporting. The same 100,000-divergence cases complete CLEAN
under `MemoryMax=512M`: 0 gating divergences, 0 advisory geometry differences,
425,056 KiB maximum RSS, and 11.26 seconds wall time.

The predecessor functions `step_once`, `alignment_step_once`, and
`step_with_observer_once` are deleted. Active integration tests pin their
absence and cover state, output, typed evidence, and observation-independent
resource suspension. Diagnostic incremental stepping retains its captured
fatal policy at the `CanonicalStepRunner` API boundary; the unified operation
itself remains TeX82's sole fatal commit authority.

## Deletion accounting

Across implementation commits `6fba2e52d`, `7880bb5a6`, `8f6b72915`,
`46f573c6c`, and `d02056216`, authored Rust changes are 477 additions and 1,166
deletions, for 689 net deleted lines. Production executor Rust accounts for
346 additions and 833 deletions (487 net deleted); active and retired test Rust
accounts for the remainder. Documentation and guidance changes are excluded
from the Rust deletion total.

## Validation

The exhaustive command/evidence differential tracer reports zero ordered
divergences and zero advisory geometry differences across every registered
fixture. Focused executor and incremental suites, exact DVI conformance,
resource retry parity, loaded/ordinary text-channel parity, and the enforced
snapshot allocation/runtime gate pass under their 512 MiB or 1 GiB guards.
The full native `cargo test -q --tests` suite and all four `scripts/check.sh`
gates are the final repository acceptance gates.
