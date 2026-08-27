# `umber2-66p0.35`: ErrorStop recovery transition

## Architecture deletion

TeX82 §§84/87 insertion and deletion now leave `ErrorReport::error` as the
typed `ErrorOutcome::Recovery(ErrorRecoveryRequest)` result of the synchronous
ErrorStop interaction. Processor-owned scanner reports consume that result
immediately. Executor-owned reports put the same request in the existing
operation-local `DiagnosticEffects` handoff; the canonical operation transition
then constructs the sole `CommandProcessor` input owner and consumes the
request once.

The `World`-resident pending-recovery mailbox and all eight ordinary raw and
expanded delivery polls are deleted. `get_token_into` therefore returns to
being only raw token delivery. Deletion recursively uses that same raw route,
stops at EOF, renders the post-delete context, and resumes the existing dialog;
insertion opens exactly one error-insert line and returns. Processor-side and
executor-side reports preserve the same typed jump-out branch.

This reduces representations from an interaction result plus a generation-
resident queued action to one typed result and, only when an executor must
release its state borrow, one attempt-local `Option<ErrorRecoveryRequest>`.
There is no cache, special fast path, second input route, heap indirection,
compaction, or new lifetime owner. The explicit executor propagation is the
unavoidable mechanical fallout: 17 `tex-exec` production files now pass the
operation-local diagnostic handoff to error-report helpers. The semantic
change also touches ten `tex-command` production files and three `tex-state`
production files; macro scratch and fuel ownership are unchanged.

Focused tests exercise typed two-digit deletion and inserted-line outcomes.
Existing executor tests retain live-context-before-prompt ordering, raw token
deletion, one-shot inserted source delivery, recursive interaction, jump-out,
alignment interception, suspension, rollback, and diagnostic publication.

## Authenticated boundary

The exact comparison uses arXiv `2606.12566`, selected `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format object `ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
and packed distribution root `721e833071d92bba` whose `manifest-v8.json` has
SHA-256
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`.
Both ELFs use `RUSTFLAGS='-C force-frame-pointers=yes'` and the profiling
profile. The exact-base `ef836b425` ELF is 387,386,176 bytes, has SHA-256
`22c8b081740f64b0baf2a025fb6bf58a5da0c2e1aaa4b233b24fa1e8878f03bd`,
and build ID `b9ea937d7a0009b98b93734d7ee9a2847ccf814a`. The candidate ELF is
387,550,416 bytes, has SHA-256
`e53b3cd05d83d54bcd7a3d8b518df1c0310a9f9c70d4126647259193b9974a94`,
and build ID `2ae4af754f4d94d57fd0bf4ed9f23b70278530b3`.

Every accepted row ran under `flock /tmp/umber-perf-host.lock`, began and ended
with CPU `some` and `full` pressure `avg10=0.00`, and had no Cargo, rustc,
Umber, or perf peer. Rows interrupted by Ansible pressure remain explicitly
rejected in issue-private evidence. All accepted rows stopped intentionally at
status 1 with the exact vector
`(20000000,19913119,2218327,6020965,16785710,4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Standard output is empty, no partial PDF is published, and
the warmed rows acquire no resource.

## Quiet paired CPU result

Three alternating exact `perf stat cycles:u,task-clock` pairs produced these
hardware-counter results:

|   Pair | Baseline cycles | Candidate cycles | Change | Baseline task clock (ms) | Candidate task clock (ms) | Change |
| -----: | --------------: | ---------------: | -----: | -----------------------: | ------------------------: | -----: |
|      1 |  18,546,068,666 |   17,271,811,355 | -6.87% |                 7,879.82 |                  7,282.21 | -7.59% |
|      2 |  18,111,424,494 |   17,619,249,466 | -2.72% |                 7,653.47 |                  7,398.64 | -3.33% |
|      3 |  18,169,650,187 |   17,258,081,485 | -5.02% |                 7,642.78 |                  7,298.11 | -4.51% |
| Median |  18,169,650,187 |   17,271,811,355 | -4.94% |                 7,653.47 |                  7,298.11 | -4.64% |

Median wall time moves from 7.91 to 7.52 seconds (-4.93%), median user time
from 8.62 to 8.19 seconds (-4.99%), and median peak RSS from 297,216 to
297,488 KiB (+272 KiB, 0.09%).

## Zero-loss owner cycles

The accepted 199 Hz captures contain 1,428 baseline and 1,484 candidate
samples. Their weighted totals are 18,054,588,280 and 17,506,620,130 cycles,
respectively. Neither raw stream contains a `PERF_RECORD_LOST` event and both
symbolization stderr files are empty.

`CommandProcessor::apply_error_stop_recovery` falls from 385,868,227 disjoint
self and ancestry cycles (2.14% of the baseline capture) to zero. The candidate
retains the symbol because it is the real ErrorStop transition, but the exact
20M workload never enters an ErrorStop dialog; ordinary delivery no longer
calls it. This is complete removal of the measured polling owner, not a rename
or an inlined hidden copy.

## Verification

The focused `tex-state` print tests pass 12 cases; focused `tex-exec` ErrorStop
tests pass the three insertion/deletion cases and the context-order case. The
complete `cargo test -q --tests` routine suite passes. `scripts/check.sh`
reports all dprint, Biome, rustfmt, and both clippy-resolution gates clean. No
follow-up issue was discovered.
