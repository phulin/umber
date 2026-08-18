# umber2-awgc.4.4: Journaled Transaction Cutover Validation

## Status

Promotion is blocked. The source audit and focused transaction controls pass,
but clean main does not reach the immutable 12,000,000-fuel arXiv endpoint and
the exhaustive Gentle command trace is not clean. The blocking defects are
`umber2-awgc.15` and `umber2-awgc.16`; neither gate is weakened here.

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

## Pinned evidence and blockers

The immutable source, schema-11 format, schema-3 distribution, ordered 105-key
closure, and offline 528-file cache match the SHA-256 authority in
`umber2-awgc.12`. The accepted `.12` binary reaches fuel with 12,000,000 fuel
charges and 11,999,815 token-frame steps. A freshly rebuilt clean-main
`8a46fd3c0` binary instead stops after the first resource rollback with
`emergency-stop(End of file on the terminal!)`; its causal digest is
`987c4bc01cbc0258b8073d5838d9cc6f455538ab38a3f5eb0abf97c2e0d51676`.
The same result appears at a reduced 100,000-fuel bound. Independently
disabling the new direct journal retirement and private-suffix cleanup leaves
that exact fingerprint unchanged, so `.4.4` did not introduce it.

The partial profiling row reaches 68 delivery/scan boundaries and one resource
rollback before that defect. Within the observed prefix, command-state and
step-snapshot clone calls, logical bytes, and allocations are all zero;
step-snapshot phase entries are zero; both internal-lineage stop counts are
zero. This is useful structural evidence but is not substituted for the full
pinned endpoint. The clean-main exhaustive tracer also reports one Gentle
backup-input retirement divergence at oracle event 204893. The corresponding
defects block `.4.4` and parent `.4` from closing.
