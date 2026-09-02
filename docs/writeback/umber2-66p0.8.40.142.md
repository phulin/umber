# `umber2-66p0.8.40.142`: residual cold scanner context borrowing

## Adopted boundary

The residual transaction/cold path in `dispatch_typed_operation` no longer
materializes a 240-byte `CommandContext` through
`Universe::command_context().expect(...)`. The direct-episode caller already
knows whether a dependency region is active, so that scalar fact now reaches
the private typed-dispatch entry directly. A tracked operation computes its
detached executor-mode fingerprint before admission; one
`Universe::with_command_context` callback then reborrows the admitted context
through tracked projection, TeX82 §1030 main-control entry, command processing,
and operand-scanner settlement.

The callback ends before pending-diagnostic reporting, immutable-resource
resolution, cold-operation preparation and application, suspension packaging,
host effects, or rollback. Host-owned nested accent and math-choice execution
propagates only the tracking boolean into its nested command episodes. No
context, host fact, scanner facade, cache, box, or second lifetime owner crosses
those boundaries.

## Exact evidence

The authenticated 20,000,000-fuel arXiv `2606.12566` workload used the schema-12
format object `ahash64-v1-2b924b5bba05d8a0`, TeX Live 2026 distribution aHash64
`df66c327ae636145`, fixed source epoch `1787080434`, and the same ordered
123-key input closure as `.140`. Exact-base binary SHA-256 was
`0b4fd45a45a5d0996912b3d7268d20f6420ae7ca4e352aa7634f828b756c9205`;
candidate SHA-256 was
`2719794e7e087900e2720644d52da5093b075888ae5cafe6da849b3a693486c1`.

Both runs stopped at the identical command-work vector
`(20000000,19907047,2216877,6018482,16781922,4011)`. The exact-base public-copy
census attributes 33,886 `memcpy` calls and 8,132,640 bytes to
`Result::expect` under `dispatch_typed_operation`, each exactly 240 bytes. The
candidate symbolized census contains no `dispatch_typed_operation`,
`Result::expect`, or other `CommandContext` owner row: the precise irreducible
remainder is zero. Both probes reported zero overflow. Issue-private raw and
symbolized receipts are under `target/umber2-66p0.8.40.142/{baseline,candidate}`.

The full hot-core census, including every named allocation owner, is
byte-identical between exact base and candidate. The profiling-only 1/4,096
warmed resident-cold-scan gate also records zero allocation calls and bytes,
zero address changes, and zero overlapping moves.

Seven alternating no-probe `cycles:u,instructions:u` pairs used the same two
binaries and warmed cache. Candidate instructions improved in six of seven
pairs: mean 28,081,120,363 to 28,054,559,065 (-0.09%), with median paired delta
-0.11%. Candidate cycles improved in five of seven pairs: mean 15,218,150,380 to
15,028,155,072 (-1.25%), with median paired delta -1.14%. The mixed cycle pairs
make no latency claim; they record paired context for the exact structural copy
removal. Pair receipts are under `target/umber2-66p0.8.40.142/ab/`.

## Semantic and validation coverage

The existing `tex-exec` suite covers normal scalar and structured completion,
nested math-choice and accent scanning, input/font/PDF resource suspension and
retry, scanner recovery, named-boundary and `afterassignment` publication
order, resource failure, and direct-operation rollback. The source audit now
also proves that residual tracked projection and scanner dispatch share exactly
one callback-scoped admission and contain no `stores.command_context()` call.

`cargo test -q --tests -p tex-exec` passed 760 unit tests with two ignored, four
additional tests, and 24 integration tests. The focused profiling allocation
gate passed with the `profiling` feature. Final workspace and repository gate
results are recorded in the Bead close reason.
