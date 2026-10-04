# Native meaning words

The former native `MeaningWord` was a 24-byte Rust enum. Its tag was separate
from the scalar command byte; the font variant contained a complete 16-byte
runtime identity. Reading an ordinary scalar therefore copied a larger value
and distinguished two command representations before execution.

The native replacement is a 16-byte value with two machine words and the
existing generation brand. Its first field is the actual command word; the
second carries an opaque coordinate only when the command requires one. It
introduces no additional registry, allocator, ownership counter, cached opcode
or command-processing front.

| Meaning | Command word                                | Coordinate word                      |
| ------- | ------------------------------------------- | ------------------------------------ |
| Scalar  | Existing opcode, flags and operand          | Unused nonzero sentinel              |
| Macro   | Macro opcode and its complete flag byte     | Complete opaque definition reference |
| Font    | Font opcode, allocation generation and slot | Complete allocation namespace        |

Live fonts already have at most 32,768 slots. Their 15-bit slot and 32-bit
allocation generation fit in the command word's 48-bit operand. The complete
64-bit namespace remains in the coordinate word. No namespace is shortened,
no font identity is reconstructed from a bare slot, and foreign or stale
identities remain distinguishable after rollback and candidate forks.

The coordinate field is nonzero, retaining the definition reference's existing
invariant without validating it again on each macro read. Scalar commands use
one canonical unused sentinel in that field. The definition reference remains
complete. Actual meaning cells, native save records, independent checkpoint
preimages and accepted alternates retain the
same counted definition roots. Reading and copying a word acquires no root.
The aggregate restore order and font/definition timeline owners are unchanged.

A scalar whose raw opcode collides with a coordinate opcode uses the reserved
raw-scalar header and keeps its complete scalar word in the coordinate field.
This represents the existing scalar value domain within the same codec. It
is not a fallback interpreter, old-format importer or additional delivery
path. Ordinary scalar words need no such projection.

Dense lookup copies the stored native value directly into the final consumer
slot. The generic command-target trait, native-row decoding writer and separate
outer-meaning probe are deleted. Fonts and macros are never unpacked and
reassembled solely for delivery. Profiling counts actual dense accesses, without
counters for deleted decoding or nonexistent clone operations.

The old enum and its pattern-matching API are deleted. Consumers read actual
scalar words, macro coordinates or font identities through the new value's
projections; they must not introduce a decoded native-kind enum that is created
and immediately consumed. Decoded `Meaning` and `ResolvedMeaning` remain at
the existing demanded diagnostic and observation boundaries.

Expansion tests the actual native family byte and reads a primitive's real
operand directly. Character scanners read the retained character/category
payload. Neither request constructs a scalar-word projection and a static
command class solely to classify the same value again. Terminal dispatch
returns a scalar key without an optional wrapper or fallback font selector.

Compared with the current-input/current-mode owner implementation, the retained
direct projections reduce matched INITEX marginal instructions from 193 to 188
for relax, 822 to 815 for a two-relax macro body, 1,231 to 1,213 for an empty
group, and 2,438 to 2,399 for a count assignment. Empty macros and argument
matching remain essentially unchanged. These are modest gains; the older
relax baseline and the clean pdfTeX reference remain faster.

Flattening every selector/category into a 16-bit stored dispatch opcode was
tested and rejected: it rearranged literal payloads, complicated complete
scalar-domain encoding, and increased ordinary delivery instructions. The
retained encoding preserves literal payloads and full scalar flags directly.
Read-only terminal entry borrows actual host facts; only a real character
flush borrows the mutable execution owners. This avoids a callback handoff
whose inlining changed when the interpreter's generated code changed.

The implementation must preserve the complete native suite, the existing
ordered canonical comparison and full checkpoint/group restoration. Matched
instruction and cycle measurements diagnose the resulting reader and macro
paths. If the replacement regresses, investigate its generated loop and fix
the representation or its consumers; no numerical acceptance threshold is
substituted for completing the redesign.

Matched INITEX marginal instruction measurements on 2026-10-04 compare this
implementation with the preceding native interpreter: relax 207 → 196,
two-relax macro bodies 881 → 848, empty macros 205 → 200, empty groups
1,288 → 1,222, count assignments 2,480 → 2,460 and empty boxes
19,573 → 18,946. The older pre-rebuild relax baseline remains 154 and the
clean pdfTeX reference 91, so the overall performance gap remains open.

Shrinking the enum alone initially increased relax cost to 236 instructions.
Deleting dense-row decomposition/reconstruction and projecting terminal and
outer selectors directly recovered that regression. The three-repeat dense
relax cycle measurement trends from 69.5 to 67.1 cycles per additional relax,
with an older baseline of 60.5 and noticeable sample variation. This is not
evidence of a decisive cycle gain. Further work belongs at the actual input
frame, terminal reader and nonempty macro frame transitions.
