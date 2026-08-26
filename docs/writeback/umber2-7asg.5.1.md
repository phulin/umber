# `umber2-7asg.5.1`: canonical command construction attribution

## Evidence boundary

The CPU authority is the immutable e1f21f7275da208ddfc6b308027f9ddaa4e981a6
capture under `target/umber2-7asg.4/`. Its archived optimized full-debug ELF
has SHA-256
`9ed990aea7d86083c2d03f30d54dfc00b0cbad27f272bc664fe8318e66af50a7`.
The zero-loss `cycles:u` capture contains 1,610 samples and
19,196,380,076 weighted user cycles. Every measured row stopped at the exact
command-work vector
`(20000000,19913119,2218327,6020965,16785710,4011)`.

The symbol
`tex_command::processor::next::get_next_canonical` owns exactly 181 self
samples and 2,215,870,854 weighted cycles, 11.54% of the process total. The
independently tracked `CurrentCommand::resolve_into` symbol owns 886,122,219
weighted cycles and is excluded throughout this note. Its call-site setup
remains part of the canonical leaf because those instructions execute before
the out-of-line call.

An issue-private `profiling`-feature census added relaxed process-local
counters, ran the same source, packed distribution, format, closure,
environment, guards, and 20M fuel boundary under the required host lock, then
removed all instrumentation. It reproduced the exact six-counter vector.
Counter writes make its time non-comparable; the immutable capture alone is
the cycle authority. Issue-private derived sample maps, the census run, and
redirected build logs are under `target/umber2-7asg.5.1/`.

The archived DWARF and disassembly establish three important layout facts:

- `get_next_canonical` is 8,825 bytes of machine code and reserves 1,208 bytes
  of stack;
- `take_input_token` is inlined into it, including all five packed storage
  domains, source tokenization handoff, replay-completion probes, retirement,
  and the success envelope; and
- `DeliveredToken` is 104 bytes. It carries a 40-byte
  `SourceProvenance` behind `Option`, even on the overwhelmingly common stored
  path where that value is absent.

## Disjoint self-cycle attribution

Each of the leaf's 181 self samples is assigned exactly once by instruction
address. The address cuts follow the archived symbol's generated basic-block
layout; the two split return/stamp tails at `0xc40f20..0xc40f6b` are assigned
back to their source operation. The table therefore sums exactly to the
authority's 2,215,870,854 cycles.

| Generated operation                                                                      | Samples |   Weighted cycles | Leaf share |
| ---------------------------------------------------------------------------------------- | ------: | ----------------: | ---------: |
| packed-token route, backing access, cursor advance, provenance fields, success packaging |      79 |       971,112,055 |    43.825% |
| top-level `InputLevel` selection and `ActiveInput` materialization                       |      28 |       343,077,501 |    15.483% |
| replay-completion readiness probe                                                        |      17 |       206,847,719 |     9.335% |
| delivery unpack, parameter test, stamp, and token-kind normalization                     |      14 |       170,786,821 |     7.707% |
| alignment classification, raw-trace predicate, and command handoff                       |      13 |       159,529,903 |     7.199% |
| backed-up/`\noexpand` treatment test                                                     |       8 |        96,464,869 |     4.353% |
| command-work counter updates                                                             |       7 |        83,265,664 |     3.758% |
| argument setup for the separately attributed `resolve_into` call                         |       5 |        62,664,260 |     2.828% |
| 1,208-byte-frame return epilogue                                                         |       5 |        61,946,255 |     2.796% |
| delivery-stamp publication and outer-validity entry test                                 |       5 |        60,175,807 |     2.716% |
| **Total**                                                                                | **181** | **2,215,870,854** |   **100%** |

The first bucket deliberately keeps provenance copies and the returned
success envelope together: LLVM emits the same move instructions for the
composite value, so splitting their sampled cycles would fabricate precision.
The independent frequencies below show how rarely the wide provenance member
is populated.

## Exact operation frequencies

`get_next_canonical` entered 19,913,304 times. It delivered 19,913,119 final
commands and 185 end statuses. No replay-completion status surfaced in this
prefix. Before final command construction, `take_input_token` produced
20,896,077 candidate tokens; 982,958 parameter markers, 4.7040%, redirected
to argument replay and never needed a `CurrentCommand`.

| Candidate source                |     Deliveries | Candidate share |
| ------------------------------- | -------------: | --------------: |
| macro replacement               |     10,803,804 |        51.7025% |
| macro argument                  |      7,924,568 |        37.9237% |
| replay lane                     |      1,652,596 |         7.9086% |
| physical source                 |        463,231 |         2.2168% |
| attempt list                    |         45,879 |         0.2196% |
| durable list                    |          5,908 |         0.0283% |
| synthesized v-template sentinel |             91 |         0.0004% |
| **Total**                       | **20,896,077** |        **100%** |

The storage path selected a token level 22,223,261 times and a source level
463,517 times. Of the token-level selections, 1,790,506 discovered depletion
and retired or restarted the level. Thus the common path repeatedly constructs
an `ActiveInput`, re-borrows the top level, routes a 40-byte
`PackedTokenSpanHandle`, advances the fixed frame, packages the 104-byte
success value, unpacks it in the caller, and only then decides whether the
word was a macro parameter.

### Normalization and resolution handoff

The leaf decodes `spelling.semantic_token()` once for every 20,896,077
candidate in the parameter test and again for every 19,913,119 final command
in the control-sequence/active-character test: 40,809,196 token decodes before
the separately attributed resolver performs its own canonical decode. Exactly
6,020,965 final commands, 30.2362%, require a live meaning lookup. The leaf
spends 170,786,821 sampled cycles in the shared unpack/parameter/stamp/kind
region and another 62,664,260 cycles shuffling the spelling, stamp,
provenance, source flags, and destination pointer into `resolve_into`.

This evidence does not assign the resolver's table lookup, meaning clone, or
final `CurrentCommand` field writes to this issue. Those remain the complete
scope of `umber2-7asg.5.2`.

### Admission and token delivery

The five stored-domain routes account for 20,432,755 candidate tokens,
97.7828% of all candidates. Their handles were admitted when their input
levels were created; delivery performs no token, meaning, provenance, or root
admission and the symbol contains no allocator call. Only 287 first-source
registrations occurred, 14.35 per million fuel actions, before the source
cursor's `backing_registered` bit became stable. Control-sequence creation for
new physical-source spellings remains tokenizer-owned and is not a stored-path
operation.

There is therefore no evidence for a meaning cache, recent-token cache, or
per-command classifier. The repeated work is generic envelope and routing
machinery around already-admitted storage.

### Provenance

Physical source delivery produced 351,062 one-byte direct origins and 112,169
range origins. Stored delivery carried a non-unknown `OriginId` 613,165 times
and an exact `SourceProvenance` only 11,284 times. Including direct source
tokens, the 104-byte success envelope's wide source-provenance member was
present for 474,515 of 20,896,077 candidates, 2.2708%.

The correct warmed effect remains zero allocation: token spans borrow their
existing origin sidecars, one-byte source positions encode directly, and only
the source tokenizer may create the required range origin. Rollback ownership
continues to belong to the source map/provenance watermark and the input
frame. A delivery-local representation must not create a second provenance
arena, cache, or lifetime.

### Tracing, alignment, recovery, and expansion handoff

The unobserved production-shaped census executed 19,913,028 raw-observation
predicates and constructed zero observation records. The 91 intercepted
alignment delimiters intentionally skipped raw observation. The trace payload
therefore costs nothing when detached, but the universal predicate and its
adjacent handoff remain in the 159,529,903-cycle shared bucket.

Alignment classification ran for every final command. It recorded 1,509,127
begin-group, 1,434,189 end-group, and 91 delimiter adjustments, 14.7812% of
final deliveries in total. Outer-validity recovery occurred zero times, while
the backed-up/`\noexpand` treatment applied only 27,701 times, 0.1391%; their
tests still execute for every command and account for 156,640,676 cycles in
the two relevant generated buckets.

Replay-completion claiming was attempted at every entry, and readiness was
also checked inside every input-loop iteration, despite zero completions
surfacing in the measured prefix. The immutable profile assigns 206,847,719
cycles to that readiness region. This is an explicit executor-owned replay
boundary, not expansion semantics to be inferred from a token; its ownership
must remain structural across retirement and suspension.

## Clean-sheet default path

Replace value-returning raw token delivery with one destination-directed
input-stack operation, matching the already adopted destination-directed
`CurrentCommand` boundary:

```text
InputStack::deliver_into(raw_slot) -> RawInputStatus
  fixed top frame -> borrow backing domain -> read word -> advance frame
      parameter word -> push argument range -> restart
      ordinary word  -> fill final resolution inputs once
  source frame -> tokenize/retire through its owned side state -> same slot

raw_slot + CommandContext -> resolve_into(CurrentCommand destination)
  -> outer validity -> alignment adjustment -> optional observation -> return
```

The default token-list path should operate on a fixed-size top-frame header
whose stored payload already names its replay, macro replacement, macro
argument, attempt, or durable backing. Source cursor data and one-time source
registration remain source-frame side state. This is one storage-lifetime
dispatch selected at level creation, not a cache or inferred command
classifier.

`RawDeliverySlot` is call-local scratch owned by the active delivery request.
It is initialized in place and never wrapped in a 104-byte
`Result<Option<DeliveredToken>>`, copied into an `ActiveInput`, returned, and
destructured. A parameter token restarts from the raw word without populating
the final command. An ordinary token passes its packed word, level, position,
behavior, origin, and only-present-on-demand source provenance directly into
the caller-owned command destination. The separately owned resolver remains
responsible for meaning access and the final `CurrentCommand` layout.

The ownership contracts are:

- **Allocation:** zero after existing input/scratch capacity warmup; a source
  may still intern a genuinely new control sequence or create its required
  range origin.
- **Copying:** no returned `ActiveInput` or 104-byte `DeliveredToken`; no
  `CurrentCommand` clone; each final scalar/provenance field is written once
  to its ultimate destination.
- **Rollback:** input-frame position, source cursor, provenance watermark, and
  replay-completion frontier remain in `CommandState`; call-local raw scratch
  is discarded and needs no rollback record.
- **Suspension:** raw scratch never crosses a resource barrier. Only the same
  completed current command and typed expansion/scanner continuation may
  suspend.
- **Tracing and alignment:** outer validity, alignment classification,
  delimiter interception, and observer publication retain their present order
  after resolution. Unobserved delivery constructs no record.
- **Replay completion:** the input owner maintains an explicit completion
  frontier beside the top-frame state, so the driver reads one structural
  status rather than scanning for an inferred condition. Descendant identity
  ordering and one-shot completion semantics remain unchanged.

A focused implementation gate should preserve the exact 20M work vector and
command/corpus semantics, assert the 104-byte returned envelope is gone,
measure zero warmed allocations for mixed packed spans, and compare absolute
zero-loss self and inclusive cycles against this authority. It must report
`resolve_into` separately so work owned by `umber2-7asg.5.2` is never claimed
as canonical-construction savings.
