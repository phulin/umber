# umber2-vgjr.20 — oracle forecast reconciliation

## Owner decision

The portfolio owner accepts Program 2 at its measured 366 lines of net authored
Rust growth and retires the original 900--1,350-line reduction forecast. No
unimplemented deletion remains scheduled, and no moved, generated, historical,
fixture, documentation, or compatibility-gated lines are credited.

The forecast was based on 1,800--2,400 lines of repeated walks, finalizers,
comparators, and tests being replaced by 800--1,050 lines of shared code. The
implementation deleted 1,636 authored Rust lines and added 2,002. It correctly
found most of the forecast duplicate inventory, but underestimated the complete
schema operations, strict stateful TRIP projection, typed result surfaces, and
independent proof needed to preserve the contracts.

## Authority audit

- `tex-oracle::event` remains the only generic exhaustive event traversal
  authority. Fixture validation, transport hashing and classification,
  normalization, comparison keys, recurrence grouping, location erasure, and
  bounded rendering call its views. Retained matches in `profile.rs` validate
  the typed TRIP event set and order; they do not recreate a generic walk.
- `tex-observe::LiveSessionTranslator::finalize_once` remains the only semantic
  and geometry finalization walk. Its public detached, typed-profile, and
  encoded-session surfaces select results from that walk. Translation still
  matches engine-owned `CommandObservation` values because the engine model and
  immutable oracle schema are deliberately separate contracts.
- `tex-command-stream::policy` remains the sole host semantic-comparison owner.
  Repository replay and `event-stream-diff` use its ordinary result; parity uses
  its strict TRIP result. The strict policy's direct event matches implement
  macro lifetime, allocator-reference, instrumentation, and positional
  comparison semantics. Parity retains byte-channel precedence, bounded report
  rendering, and presentation, but no second parse, projection, or event-count
  pass.

No caller retains the deleted normalization, fixture-location, position-erasure,
parity observer/finalizer, strict projection, second JSONL parse, or separate
comparison-accounting authority. The audit found no production deletion that
has both a named surviving owner and identical behavior. Collapsing the
remaining layer-specific matches would merge contracts rather than remove
duplication.

The 380 net added proof-test lines are retained because they independently pin
all schema carriers and locations, normalization and key compatibility, typed
semantic/geometry profile bytes and provenance, strict report behavior,
malformed-stream rejection, and bounded processing of 1,000,000 events. They
are not a shadow implementation and are excluded from production accounting.

## Exact accounting

The implementation commits are `d819194fc`, `c29f2f232`, and `b3e141f75`.

| Category                  | Additions | Deletions |  Net |
| ------------------------- | --------: | --------: | ---: |
| Production Rust           |     1,588 |     1,602 |  -14 |
| Authored Rust proof tests |       414 |        34 | +380 |
| All authored Rust         |     2,002 |     1,636 | +366 |

Declarative/generated changes are four added Cargo manifest or lockfile lines;
compatibility-gated retirement and binary-fixture changes are zero. Program 2
therefore closes at the measured authored-Rust result without a forecast credit.

## Independent verification

Fresh exact-tree verification of `9b0fedead9c1e0c5fc8cb809f423b7f9c2548a7b`
confirmed the three sole authorities and found no duplicate production owner.
Commit numstat independently reproduces production `+1,588/-1,602`, proof
`+414/-34`, and authored Rust `+2,002/-1,636`; moved, generated, documentation,
fixture, and historical lines remain excluded. The owner decision is therefore
evidence-backed, and `group.rs` now correctly identifies schema-owned location
erasure. With `CARGO_BUILD_JOBS=6`, uncapped focused/full builds passed; focused
execution passed under `MemoryMax=512M`, the million-event regression and full
suite passed under `MemoryMax=1G`, and uncapped `scripts/check.sh` passed all
four gates. Every command had a finite timeout; no `prlimit` was used.

## Rebase verification

After rebasing over the repaired Program 1.1 tree, fresh verification of exact
HEAD `51f98bfa9383780a64baf123465a133f06a3c205` confirmed both issue commits and
all three authority boundaries were preserved. The new base changes only the
`tex-exec` receipt-shadow harness and its documentation; Program 2 accounting
and exclusions remain unchanged. With `CARGO_BUILD_JOBS=6`, uncapped focused
and full builds passed, focused execution passed under `MemoryMax=512M`, and
the dedicated million-event regression plus full suite passed under
`MemoryMax=1G`. Final uncapped `scripts/check.sh` passed all four gates. Every
command used a finite timeout; no `prlimit` was used.
