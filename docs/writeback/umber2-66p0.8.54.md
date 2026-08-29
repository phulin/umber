# `umber2-66p0.8.54`: authoritative in-place operation frame

## Profile attribution

The inherited authenticated 50-million-command capture at source
`0ebac8a938feebd85d671b8ffd22b827527ba3b4` reports `Samples: 38K` and
`Event count (approx.): 60480227254`. Its bounded inclusive report
attributes 16.55% inclusive and 0.08% self to
`CommandProcessor::settle_preflight_command_into`, and 12.72% inclusive and
4.20% self to `MainControl::prepare_operation`.

The source-level ownership audit found two exact redundant transfers on that
ancestry. Executor preflight took the delivered `CurrentCommand` out of its
destination and passed it by value to settlement, which immediately wrote it
back into the same destination. Separately, `OperationFrame` stored a nested
six-field `PreflightCommand`; direct and diagnostic suspension extracted that
projection, then resume reinserted it into a fresh operation frame before
preparation repeated the same phase and command checks.

## Adopted boundary

`settle_preflight_command_into` now accepts an already-occupied destination and
advances it in place. `OperationFrame` directly owns the admitted command,
parked expansion, scalar phase, delivery cursor, scanner child, and partial
direct-scan state beside its prepared/application fields. Preparation borrows
that one frame. Commit clears it; resource suspension moves it intact into the
typed direct, diagnostic, or prepared-resource continuation; rollback and
resume retain the same executor and command-state boundaries.

The nested `PreflightCommand`, command-shaped preflight error carrier, retry
projection/reinsertion helper, and command take-and-reinsert settlement are
deleted. Alignment-only retries retain their existing compact typed owner.
The implementation adds no cache, alternate command representation,
per-primitive path, unsafe code, heap indirection, or ordinary-path
allocation.

## Focused evidence

- `preflight_settlement_advances_the_occupied_command_slot_in_place` proves a
  raw macro settles to its canonical replacement at the identical destination
  address with zero `CurrentCommand` clones.
- `one_and_4096_operation_frame_phase_cycles_are_allocation_free_and_scalar`
  proves one and 4,096 frame cycles request zero allocations and zero bytes,
  with exactly 2 and 8,192 scalar transitions.
- `directly_delivered_edef_resumes_its_inner_expanded_scanner` proves a
  resource-suspended substantive command retains its command, settled phase,
  scalar scanner child, and semantic output in the one frame.
- The `tex-command` and `tex-exec` architecture tests require direct frame
  fields and reject the retired nested command and preflight-error carriers.

Per coordinator direction, this intermediate branch ran only focused tests
and no broad gate or new long profile.
