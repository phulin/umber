# Main-control dispatch

The selected direct-control design is governed by
[Full checkpoints and TeX group ownership](aggregate_checkpoint_contract.md).
Main control receives packed commands from `tex-command`, selects TeX's
mode/command arm, scans its typed operands synchronously, applies that arm,
and publishes ordered evidence. Prefixes and `reswitch` operate on the same
delivered command. Host misses unwind the call and require full-root replay.

The eligibility lane and its compact handoff slot have been deleted. Expanded
fetching and the horizontal character loop's raw lookahead use the shared
command reader. Observation changes evidence production, with observed and
unobserved terminal/error equivalence exercised by
`main_control/tests/observation_equivalence.rs`.

Remaining `.15` work replaces the cold operation union and its scanner,
promotion, preparation, application, and settlement handoffs with direct arms.
Genuine typed scanner results and actual token, definition, resource, and
builder owners retain their semantic lifetimes. The redesign has no alternate
executor or compatibility dispatch. Performance measurements are evidence,
without an implementation closure threshold.
