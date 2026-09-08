# Command delivery kernel

This design moves command-delivery work to its semantic frequency while keeping
one canonical input stack and one expansion implementation. Performance evidence
comes from identical workloads on separate baseline and candidate revisions;
profiling counters are structural evidence, not production timing.

## Input ownership

The semantic input stack is the stack of resumable readers. Each source,
replacement, argument, or stored-token frame owns its logical position and the
physical cursor appropriate to its storage lifetime. Parent readers remain in
their frames while child input is active. Checkpoint capture and restoration
continue to use those existing frame coordinates.

Delivery borrows the exposed frame directly. A second cached storage tag and
its invalidation protocol are unnecessary: the frame's storage variant is the
authority. No Rust borrow survives stack mutation, nested scanning, or resource
unwind. No raw pointer or independent cursor is introduced to evade those
boundaries. Ordinary reads retain the physical macro, argument, and replay
cursors already installed in the frame.

## Delivery and expansion

Raw fetching charges fuel before input work, retries input transitions under
that same charge, resolves one token, and settles it. Source delivery is an
ordinary fetch alternative; line acquisition, exhaustion, and recovery remain
out-of-line transitions. Source positions continue to identify the token before
advancement, so backup freshness and diagnostic source geometry remain exact.

Expanded delivery separates its initial already-fetched command and optional
classification from the repeating expansion cycle. The cycle owns an occupied
compact command, dispatches its meaning, and then fetches and classifies its
successor. Macro and primitive calls may mutate input or recursively scan;
the next fetch therefore borrows the authoritative exposed frame anew.

The raw fetching kernel is shared by raw and expanded consumers. Expansion
depth is restored on every return, including resource and fuel failure. An
error clears the destination and invalidates freshness. Replay completion,
alignment events, and synthetic commands retain their existing ordering.

## Observation and settlement

The presence of an external command observer is selected at a delivery entry.
The shared kernel specializes observation-only sequencing, raw observation,
and expanded observation for that choice. The choice is valid only for the
synchronous call: attaching an observer between calls selects the observed
instantiation on the next entry.

Token-local suppression, outer validity, and active alignment remain live
semantic checks. Literal brace accounting remains necessary even outside an
alignment. TeX tracing, diagnostics, fuel, provenance required for recovery,
and checkpoint journals are not optional observation.

## Recursive expansion capacity

Web2C's `tex.ch` change to TeX82 §366 and e-TeX's `etex.ch` [53a]
share one `expand_depth_count`: primitive expansion and `scan_expr` each
enter it, and return restores it. `get_x_token`, sequential macro calls,
and expression parentheses do not themselves add depth. Use pdfTeX's
default limit of 10,000, rejecting an entry that would reach that limit.
Keep the counter in the borrowed processor episode, outside snapshots.
All Rust error returns must restore it, including resource replay and fuel
exhaustion. This corrects the previous expression-only counter.

Native stack capacity is a separate platform constraint. As the Web2C manual
documents, a small native stack can exhaust before the semantic limit.
Routine tests exercise finite nesting and exact boundary accounting with an
already-active parent depth; explicit scaling tests need a sufficient native
stack and must expect a capacity error once the default limit is reached.

Run the explicit full-capacity audit with
`cargo test -q -p tex-command full_default_expansion_capacity_on_a_sufficient_native_stack -- --ignored --exact processor::expand::tests::nesting::full_default_expansion_capacity_on_a_sufficient_native_stack`.
It checks 1,024 genuinely nested scans and the default capacity boundary for
all five operand families on a dedicated 256 MiB virtual native stack.
This is separate from the routine tests of exact boundary accounting.

## Further span admission

Source-character and balanced-argument consumers already borrow spans. Further
resident character admission should build on those owners, stopping at semantic,
consumer, fuel, provenance, and storage boundaries. This is separate from
removing the duplicate reader selection: scalar delivery must remain correct
without span admission, and changing a callback protocol requires its own
executor validation.

## Validation

Command tests cover source and resident delivery, nested macro/argument return,
live meaning changes, suppression, raw and expanded observation, input rollback,
and resource/fuel failure. Compare observed and unobserved semantic outputs and
fuel without conflating optional record production with TeX work. Run focused
tests, the native routine suite, and the authoritative `scripts/check.sh` gate.
Report missing pinned assets and pre-existing failures explicitly.
