# `umber2-66p0.37`: remaining copy and scan ownership audit

## Evidence boundary

There is no exact combined census or profile for integrated-main checkout
`432997cf9e8fadb316d2b703cf74147f6611f9ba`. This audit therefore uses the
latest committed exact combined authority, `b1279a623cd7f0e5f1cd941c001237ce8deabf00`,
documented in [`umber2-u2io`](umber2-u2io.md). Its one frame-pointer ELF has
SHA-256
`1c5049f77c30039b09deb53d6c14647ced66a72aba9186433a96ff8d60c90935`,
build ID `8a2df7cae597f610d8088c86b1b09427e5a9a59b`, and a zero-loss 1,429-sample
profile containing 16,849,307,112 weighted cycles. Its exact public-copy
census records 33,535,478 `memcpy` calls / 4,457,104,526 bytes and 51,948
`memmove` calls / 4,767,012 bytes, with zero caller or size overflow.

Every measured row stopped intentionally at the exact authenticated vector
`(20000000,19913119,2218327,6020965,16785710,4011)`. The selected source,
schema-12 format, packed distribution root, and ordered 123-key closure have
SHA-256 identities
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
`4d3887c289078e2bc0f88e96f4e77989dc38522a59c230fa5be94dffa1c7cff9`,
and `e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

Later differential reports show that integrated `umber2-66p0.34` deleted the
processor fuel representation and `umber2-66p0.35` deleted ordinary-delivery
ErrorStop polling. The in-flight `umber2-66p0.33` owns macro-match slots,
argument facts/ranges, delimiter segments, and commit machinery. This audit
does not subtract those changes from the older combined counts, and excludes
all three owners. It also excludes the parallel `umber2-66p0.36` audit's
error-context projection, command-context admissions, and file-framing event
queue. The three candidates below remain present in current source and do not
depend on those representations, but their final order must wait for the next
exact combined profile.

## Exactly three deletion candidates

### 1. One preflight command owner, retained only on suspension

**Evidence.** `MainControl::preflight_replay_delivery` owns 4,388,220 exact
`memcpy` calls / 766,753,944 bytes across mixed 136-, 144-, 160-, 208-, 288-,
and 432-byte values. The 136/144/288-byte rows are 3,689,251 calls /
621,343,048 bytes, an intentionally conservative ceiling for command and
wrapper traffic rather than a claim that every byte is removable. The exact
lower bound is the two named whole-`PendingPreflightCommand` rebuilds:
`with_cursor` and `with_scanner` each own 168,283 calls / 24,232,752 bytes at
144 bytes, or 336,566 calls / 48,465,504 bytes together. Dropping the same
pending enum owns 12,237,796 sampled self cycles. The 208-byte command-context
rows belong to `umber2-66p0.36` and are expressly outside this candidate.

**Representation to delete.** Ordinary `OperationDelivery` variants and a
speculatively cloned `PendingPreflightCommand` simultaneously represent the
same command. `for_delivery` builds the retry mirror before scanning; cursor,
scanner, and expansion updates then reconstruct that whole mirror even when
the operation completes synchronously.

**Single-owner replacement.** Put the one live command plus a compact
raw/settled/expanding phase, cursor, and scanner coordinate in the existing
attempt-owned `OperationFrame`. Delivery, classification, and operand scan
borrow or consume that slot. Completion clears it immediately; only a real
suspension transfers its occupied fields into retained typed retry ownership.
Non-command delivery tags remain compact statuses rather than a second
command-bearing enum.

**Semantic risks.** The implementation must preserve main-loop raw lookahead
versus expanded observation, `goto reswitch`, command tracing and diagnostic
order, alignment interception, the delivery cursor and scanner child's ABA
identity, prefix and scalar resume phases, resource-failure ownership, and
aggregate rollback. A clone cannot merely be delayed if a failure still needs
the exact pre-scan command; each phase must state which owner is live.

### 2. Transfer the semantic-diagnostic buffer as one owner

**Evidence.** `CommandProcessor::take_semantic_diagnostics` owns exactly
168,727 public `memcpy` calls / 33,745,400 bytes, all 200-byte values. Current
source drains `CommandState`'s `Vec<CommandSemanticDiagnostic>` and collects
the same detached values into a second `Vec` before executor reporting.

**Representation to delete.** Delete the second per-element diagnostic
buffer created by `drain(..).collect()`. It duplicates ownership transit even
though reporting does not need the producer's allocation to remain populated.

**Single-owner replacement.** At the completed processor-episode boundary,
move the existing `Vec` owner wholesale to the executor and leave the command
queue empty. The executor consumes that allocation in detection order. The
next command episode remains the sole producer; no diagnostic value is copied
between element buffers and no additional queue or registry is introduced.

**Semantic risks.** The transfer must remain after context capture and before
the established trace/error publication barriers. Aggregate rollback must
restore the pre-step queue, retry must not duplicate a transferred report,
snapshot quiescence must still reject pending diagnostics, and ErrorStop
interaction must consume exactly the corresponding report. Allocation
measurement must prove that moving the existing allocation merely relocates
the current per-episode allocation rather than adding a second one.

### 3. Compare `\ifx` through the two existing command owners

**Evidence.** `evaluate_ifx` and its operand-delivery closure own exactly
379,400 public `memcpy` calls / 68,292,000 bytes across one 288-byte and three
144-byte rows during 94,850 evaluations. The zero-loss profile assigns
12,093,032 self cycles and 108,853,146 complete-ancestry cycles to
`evaluate_ifx`.

**Representation to delete.** After raw delivery has already placed both
operands in caller-owned `CurrentCommand` slots, `evaluate_ifx` consumes them
into two owned `ResolvedMeaning` values. The macro case then clones both
definition owners again solely to borrow their parameter and replacement
token lists.

**Single-owner replacement.** Keep both commands in their existing local
slots through the predicate and compare borrowed meanings. For macros, resolve
borrowed definition coordinates and compare the canonical parameter and
replacement slices directly. No meaning summary, equality cache, or retained
reference crosses the call.

**Semantic risks.** Preserve raw `get_next` delivery with
`no_new_control_sequence`, the temporary normal scanner status that makes
outer operands legal, undefined-control equality, no-expand active-character
identity, macro flags, and token-content equality rather than definition
allocation identity. Both operands must remain resolved at their original raw
delivery boundary, and observation, suspension, and scanner-status restoration
must keep their present order.

## Recommendation and disposition

`umber2-66p0.41` is the first validation candidate because it deletes a
parallel command/retry representation at the largest surviving measured
boundary and makes the default operation path singular. This is a provisional
implementation order, not a final performance ranking: the required next
combined census may change the order after fuel, recovery, and macro-storage
deletions coexist.

`umber2-66p0.42` and `umber2-66p0.43` track the other two independent
deletions. All three reduce representations and handoffs without proposing a
cache, special fast path, heap indirection, compaction pass, generation-long
retention, or new lifetime machinery. This investigation changed no
production source and did not build, test, or benchmark.
