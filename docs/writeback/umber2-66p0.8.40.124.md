# `umber2-66p0.8.40.124`: make the macro-argument cursor absolute

## Selection authority

The integrated `.123` writeback and its authenticated 20,000,000-command
capture are the selection authority. That execution stopped at the exact work
vector `(20000000, 19907047, 2216876, 6018541, 16781945, 4011)` for fuel
charges, token-frame steps, expanded deliveries, meaning lookups, scanner
tokens, and write expansions. Its raw deliveries were source `463197`,
stored/body `11520843`, macro argument `7922916`, and synthetic end-v `91`.

`advance_resident_command_into` led application self time at 12.23%. The
cursor-span fork arena at 2.04% is excluded dense-superblock work, and page,
shipout, and DVI owners remain excluded parity work. Within the leading
resident-delivery owner, macro-argument replay was the largest exact specialized
delivery class not already changed by `.123`: 7,922,916 words, or 39.61% of
all fuel charges. The admitted argument range already carried absolute
scratch-lane bounds, but each warm word was represented as a relative
`ResidentSpanCursor` position, converted back to an absolute coordinate with
checked addition and integer conversion, compared and loaded, and then
advanced through the generic cursor.

The `.123` public-copy authority remains 6,517,723 `memcpy` calls for
952,259,353 bytes and 126,598 `memmove` calls for 21,914,254 bytes. Its leading
copy owners are excluded dense storage and DVI/page material; no copy row
motivated this issue, and no broad profile was rerun.

## Architectural simplification

`MacroArgumentCursor` now owns the admitted range's absolute scratch-lane
coordinate directly. Sequential delivery validates that scalar against the
range, performs one fixed-chunk lookup with the existing provenance-run hint,
and increments the scalar once. The relative argument position is derived only
when canonical command stamping or diagnostic projection needs it. This removes
the warm relative-to-absolute representation transition rather than adding a
threshold, cache, or special delivery branch.

The row still carries the same argument slot, input identity, optional active
source, and provenance run. First-touch history journals the exact absolute
coordinate and the provenance run together; rollback swaps both back into the
live row. Argument-set ownership, scratch retirement, source inheritance,
suspension, and the relative delivery coordinate visible outside the row do not
change. The code remains safe Rust and introduces no second backing owner.

## Focused before/after gate

The baseline was current-main commit
`b2418b4791437d088c6a1a42e30964e25e5d6aff` (tree
`7cb10ac582638a74bc9c1fe0a7d590f074f48c96`). Exact profiling binaries were
built before and after the change. The primary focused row is the production
`mixed_macro_resident_pipeline`, not a synthetic replacement for delivery. Both
binaries report the exact vector `macro_body=2000000`, `parameters=1000000`,
`replay=1000004`, `raw=2000004`, `expanded=1000000`,
`macro_expansions=1000001`, `suspension_in=0`, `suspension_out=0`,
`command_copies=0`, with zero warmed allocations and requested bytes.

| Counter                              |      Baseline |         Final |                  Delta |
| ------------------------------------ | ------------: | ------------: | ---------------------: |
| User cycles                          | 1,913,664,828 | 1,837,654,517 |   -76,010,311 (-3.97%) |
| User instructions                    | 2,561,761,484 | 2,541,759,917 |   -20,001,567 (-0.78%) |
| Internal elapsed nanoseconds         |   885,839,781 |   777,803,171 | -108,036,610 (-12.20%) |
| Nanoseconds per macro-body word      |        442.92 |        388.90 |       -54.02 (-12.20%) |
| Warmed allocations / requested bytes |         0 / 0 |         0 / 0 |                  0 / 0 |
| Public `memcpy` calls / bytes        | 130 / 346,948 | 130 / 346,949 |                 0 / +1 |
| Public `memmove` calls / bytes       |         2 / 0 |         2 / 0 |                  0 / 0 |

The exact 20,001,567-instruction and 76,010,311-cycle reductions are the
primary CPU result. The one-byte `memcpy` difference is startup layout noise;
calls are unchanged and no work moves to `memmove`. Both symbolized reports
reconcile with zero overflow, collision, or probe-internal calls.

The secondary five-million-word long-argument row preserves 305 retirements,
one rollback, and checksum `8814743616`. Its instructions are essentially
unchanged (`248694488` to `248694371`), so it is retained as a structural and
copy guard rather than used to claim the production improvement.

## Validation and evidence

`cargo test -q --tests -p tex-command` passes tex-command's 384 unit and 23
boundary tests. The resident cursor layout test reports argument cursor size 48
bytes. `cargo check -q -p tex-command --features profiling` passes. The full
format and clippy verdict is recorded by `scripts/check.sh`.

Ignored focused evidence is under
`target/umber2-66p0.8.40.124/focused-gate/`. Baseline/final production counter
receipts have SHA-256 values
`11edb04897599d5a8700e99a775e4977272d0464f7709140736d63f77437dbcc`
and `e53f62648bf14e23df62bb71cfb441d6fae60e9a77a02573f1487b194c077e39`.
Baseline/final production copy reports are
`1ea6f2ce3bd8cc047a8b96bdb677babe077623454e2ff98514094f5b8496e57b`
and `b10eb2715393495f1f8944306028135d280e13bbecffe093759beeed564d75a2`.
The exact baseline/final binaries are
`a9afffb814474780e8405a6e82f4cf17c1f7bd8eb8fde82c0e4aa5204cd3822e`
and `4226f1bc83bb61011e97106c929d25d9a008f22435a4db11b576e9ba6f285c51`.
