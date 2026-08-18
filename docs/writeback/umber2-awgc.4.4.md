# umber2-awgc.4.4: Journaled Transaction Cutover Validation

## Status

Promoted on main at `12ad19ace`. The source audit, focused transaction
controls, immutable 100,000- and 12,000,000-fuel arXiv endpoints, exhaustive
command stream, full native suite, and repository gates all pass. The two
independently reduced blockers were fixed without restoring an aggregate
retry authority.

## Residual audit

Production `StepSnapshot`, `LocalRetrySnapshot`, aggregate host-retry methods,
and episode `InternalStop` vocabulary are deleted. Active alignment,
diagnostic expansion, ordinary execution, and resource continuations all use
the direct operation path. `DirectOperationMark` is a fixed-size,
non-restoring environment-journal cursor plus private-allocation watermarks. It
owns no aggregate rollback root. Level-zero direct commits retire environment
history only when the current operation changed it and no checkpoint, group,
dependency region, or fork prefix retains the baseline.

The schema-1 `step_snapshot` allocation, clone, and phase slots and the
`internal_group_lineage`/`internal_rollback_lineage` stop slots remain only as
frozen comparison fields. No production caller can increment the snapshot
slots or emit an internal stop.

## Focused controls

- 10,000 level-zero direct commits retain zero environment-journal entries and
  a constant retained capacity.
- A no-op direct operation does not retire earlier setup history or alter the
  named-checkpoint hash schedule.
- Open groups, retained checkpoints, dependency regions, and fork prefixes keep
  their exact restoration authority.
- Private token, macro, and glue rejection truncates only the unpublished
  operation suffix; committed roots remain selectable for revision acceptance.
- `production_batch_keeps_ordinary_prefix_on_resource_need`, including a
  private revision, proves that one resource rollback does not replay the
  committed ordinary prefix.

Capture is O(1) in live-state size: the mark contains scalar journal and store
watermarks and registers no root. Rejection is O(discarded suffix), implemented
by `Vec::truncate`/weak-slot reclamation in the three migrated private value
stores. Environment rollback remains O(changed cells) through the inverse
journal.

## Pinned evidence

The immutable source, schema-11 format, schema-3 distribution, ordered 105-key
closure, and offline cache match the SHA-256 authority in `umber2-awgc.12`.
The final profiling binary reaches the required endpoint with 12,000,000 fuel
charges and 11,999,815 token-frame steps. Stdout is empty; the typed status is
the expected fuel-exhaustion failure; and no PDF or input-record artifact is
published. The guarded profiling process takes 48.23 wall seconds and peaks at
873,660 KiB RSS. These are phase `.4` validation measurements, not the final
epic performance target.

The full row records zero command-state clone calls, zero step-snapshot clone
calls, zero logical clone bytes, zero snapshot allocation calls or bytes, zero
step-snapshot phase entries, and zero internal group- or rollback-lineage
stops. It crosses 129,903 delivery/scan boundaries, 129,812 semantic-apply
boundaries, and 111 typed resource rollbacks. The secondary work vector is
`(1,177,349 expanded deliveries, 3,506,292 meaning lookups, 10,599,869
scanner-status tokens, 1,182 write expansions)` under the versioned
direct-prefix contract in `umber2-awgc.12`; fuel and raw token-frame position
remain exact, while eliminated or retained replay is not synthetically
charged.

`umber2-awgc.15` fixed the resource blocker in `46ba1b765`: TeX82 §§440--445
expanded integer and alphabetic-constant scans now retain typed leading,
radix-tail, and completed-character continuation state across host suspension.
`umber2-awgc.16` fixed the independent Gentle trace blocker in `12ad19ace`:
TeX82 §§1123--1124/1270 nested accent assignment dispatch now hands the settled
font command through without an extra expanded observation. Neither fix adds a
snapshot, fallback executor, or counter adjustment.

The exhaustive `tex-command-stream` comparison reports `VERDICT: CLEAN` after
comparing every registered fixture through Gentle to exhaustion: zero ordered
gating divergences and zero advisory geometry differences. The focused
command, state, and executor suites pass; `cargo test -q --tests` passes; and
`scripts/check.sh` reports all four gates passed.
