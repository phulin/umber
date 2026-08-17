# Episode Deletion Boundary Audit

Status: deletion is blocked on profile-wide canonical episode coverage.

Issue `umber2-64v2.16` is the final convergence stage selected by
[`engine_architecture_decision.md`](../engine_architecture_decision.md). The
audit at `640987aa4` found that the required cutover has not happened across
the supported TeX82, e-TeX 2.6, and pdfTeX 1.40.29 profiles. Deleting the live
scalar command machine now would delete required semantics. Deleting the
packed episode instead would discard the exact `.12` performance result. A
fallback rename or an unobserved-suite zero count would conceal the gap.

## Current authorities

The following are canonical production owners and remain in scope:

- `EngineSession` is the retained host lifecycle and `MainControl` is the sole
  public execution front.
- `CommandState` and `CommandProcessor` own the complete tokenizer, source and
  token-list input stack, raw and expanded delivery, macro matching, scanners,
  conditions, and alignment delivery for all supported profiles.
- `Universe` owns the dense environment backing, typed group journal, exact
  identity, dependency state, effects, and the aggregate local-retry snapshot.
- `NodeListBuilder` is the sole mutable node builder. `NodeListRef` is the sole
  published node-list owner.
- `EpisodeCommit`, `EpisodeCommitBoundary`, `SemanticEpisodeBarrier`, and the
  fixed operational telemetry describe required boundaries of the same live
  engine.
- Committed command fixtures, property catalogues, focused adversarial tests,
  selected corpora, TRIP/e-TRIP, and artifact/DVI/PDF fixtures remain external
  evidence. They are not executable substitutes for production semantics.

## Remaining migration-only path

`tex_command::NativeBatchProgram` is not yet the canonical command machine.
After `umber2-uvfm` it owns no source bytes, admitted token vector, tokenizer,
cursor, input frame, macro body or argument. It retains only a capacity hint
and consumes the production `CommandProcessor` source/token-list stack,
expanded delivery, backup, macro/argument, and alignment-template levels. Its
remaining private integer scanner and `Control` dispatcher recognize this
smaller literal control vocabulary:

```text
count advance global shipout hbox kern relax end begingroup endgroup
```

Canonical expansion now supplies ordinary macro meanings; there are no fixed
benchmark macro slots. State access remains limited to the count bank and
hbox/semi-simple group journal. Node construction accepts only character `A`
and explicit kerns inside one shipped hbox. The executor then synthesizes the
root box and uses the ordinary shipout transaction.

`EngineSession` registers every root once in `CommandState` and retains this
capacity-only episode marker. Execution refusal restores the full
command/state/mode aggregate and resumes ordinary dispatch on the same input
stack. Source delivery and expansion therefore have one owner; scanner and
dispatch coverage fallback remains migration debt.

The benchmark-local whole-job scalar comparison adapter and its
`canonical|production|compare` selector are deleted by this audit. The
production workload remains as the exact 6M/12M/nested performance gate;
correctness stays with external fixtures and oracle workloads. The unused
`NodeOrFont` fallback-family counter is also deleted.

## Exhaustive fallback inventory

| Family               | Producer and present boundary                                                                                                         | Supported-profile gap                                                                                                                                                                                                                    | Deletion proof required                                                                                                       |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------- |
| `CharacterProfile`   | no producer                                                                                                                           | Exact TeX82, e-TeX 2.6, and pdfTeX 1.40.29 profiles all enter through their production `CommandState`; there is no episode character-mode selector. UnicodeExtended remains outside this issue's exact-profile acceptance set.           | Retain the zero counter until the final telemetry deletion pass.                                                              |
| `SourceTokenization` | no producer                                                                                                                           | Closed by `umber2-uvfm`: physical roots, registered files and streams, terminal and `\scantokens` input, category changes, backup/noexpand/template levels, suspension, retirement, and root completion use one `CommandState` stack.    | Retain property, fixture, adversarial, and selected-corpus zero evidence until final deletion.                                |
| `CommandVocabulary`  | every meaning outside the retained ten-control dispatcher                                                                             | Nearly the complete TeX82 vocabulary and every e-TeX/pdfTeX extension reject. Resource/effect/diagnostic meanings are separately classified as required barriers, but that is not family coverage.                                       | Exhaustive static profile dispatch plus property, fixture, adversarial, and corpus zero-fallback evidence.                    |
| `ScannerOrExpansion` | malformed retained integer/dimension grammar or arithmetic overflow after mutation begins                                             | Canonical macro grammar and expansion now feed the episode, but only unsigned decimal integers and count assignment/advance are consumed there. Canonical scanners, conditions, alignments, and recovery still resume ordinary dispatch. | Exact observer/event/diagnostic parity and deletion of the remaining private scanners and conditional/dispatch boundaries.    |
| `GroupLineage`       | committed ordinary-operation coverage boundary                                                                                        | This is not an admission fallback. It stops a bounded scalar episode after a group mutation so it cannot replay a committed action.                                                                                                      | Replace the implementation-only stop only after canonical group and builder publication can continue safely.                  |
| `RollbackLineage`    | committed-failure ordinary-operation coverage boundary; the packed state barrier maps here only after required-barrier classification | This is primarily scalar aggregate-lifecycle evidence, not permission to run a second engine.                                                                                                                                            | Preserve the sole aggregate retry authority; remove only migration-only boundary accounting proven unreachable after cutover. |

`NodeOrFont` had no producer: it appeared only in telemetry enumeration tests.
It was genuinely unreachable and is removed now. A missing `A` metric is a
typed diagnostic barrier, while all other material rejects earlier as command
or grammar coverage.

Required `Resource`, `Effect`, `Diagnostic`, and `Format` execution barriers
are not coverage fallback. They are recognized from canonical primitive
meanings and processor errors, but that classification alone is not evidence
that the corresponding command families execute through the retained
dispatcher.

## Evidence limitation

The `.12` and `.15` exact comparisons and 6M/12M/nested measurements are valid
for the closed workload above. They prove one state owner, aggregate rollback,
typed barriers, output equality, and the historical performance ceiling for
that slice. The canonical source/input cutover deliberately invalidates those
old performance numbers until `umber2-3gln` optimizes the now-shared expansion
and scanner machinery. It does not prove complete supported-profile command
closure.

The command-semantic runner constructs `MainControl` with
`register_root_source` and drives `step_with_observer`. An observer is a
required episode barrier, so those fixtures deliberately exercise the
complete scalar `CommandProcessor` and never attempt packed admission. A zero
fallback count from that route is therefore vacuous for command/dispatcher
coverage. It is no longer vacuous for source ownership: production
`EngineSession` always registers its root through the capacity-only episode
marker, and the episode has no alternative source/input implementation to
select. Focused tests additionally force re-entry after category changes,
registered-file and read-stream levels, `\noexpand`, alignment templates,
resource suspension, and root completion while asserting the retained source
fallback counter remains zero.

## Ordered blockers

The remaining cutover is split by semantic ownership, with dependencies in
Beads:

1. `umber2-uvfm` canonicalized source and input frames for supported exact
   profiles.
2. `umber2-3gln` depends on it and canonicalizes expansion, scanners,
   conditions, and alignments.
3. `umber2-c1p8` depends on both and canonicalizes dispatcher, state, node,
   output, effect, resource, and retry families.
4. `umber2-64v2.16` depends on that chain and performs the final static
   deletion, zero-fallback audit, complete correctness hierarchy, and exact
   `.12` performance gates.

Until those blockers close, the valid deletion boundary is narrow: delete
only adapters with no semantic caller, retain scalar semantics, and keep every
fallback explicit and counted.

## Cutover update

Issue `umber2-c1p8` subsequently removed the migration-only path inventoried
above. Every production and test root now enters the complete bounded
`MainControl` dispatcher directly; there is no command-vocabulary admission,
packed root marker, packed terminal continuation, or coverage-fallback
counter. The ten-control kernel, count/group sidecar, A/kern-only sink, and
synthesized shipout path are deleted. `umber2-64v2.16` may now perform its
final static and performance audit against the one canonical executor.

The final same-tree 10,000-call comparison changes only the public driver.
Direct scalar/episode medians are 893.157/514.320 ms, 615,016/286,436
allocation calls, 194,979,672/81,861,980 requested bytes, and 40,860/38,996
KiB peak RSS. Nested scalar/episode medians are 902.073/536.304 ms,
615,074/286,489 calls, 203,546,559/80,474,939 bytes, and 39,588/37,916 KiB peak
RSS. Exact fuel and output-work fields agree. The episode therefore recovers a
1.74x direct and 1.68x nested CPU gain while removing 53.4% of allocation calls
and 58.0--60.5% of requested bytes without retaining a second executor.
