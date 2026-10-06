# Native meaning words

`MeaningWord` is a generation-branded, 16-byte value with two machine words.
Its command word stores the actual execution key and payload. The coordinate
word carries the complete opaque identity when the command needs one. There is
no decoded enum tag, additional registry, cached opcode or delivery owner.

| Meaning | Command word                                 | Coordinate word                      |
| ------- | -------------------------------------------- | ------------------------------------ |
| Scalar  | Execution key and operand                    | Complete scalar flags plus one       |
| Macro   | Empty/simple/matching key and complete flags | Complete opaque definition reference |
| Font    | Font key, allocation generation and slot     | Complete allocation namespace        |

The execution key occupies the command word's upper 16 bits. Static scalar
families, macro shapes, literal categories and primitive selectors have disjoint
keys. Character and primitive keys directly select their semantics; ordinary
reading does not reconstruct a raw scalar to classify it again.

Live fonts already have at most 32,768 slots. Their 15-bit slot and 32-bit
allocation generation fit in the command word's 48-bit operand. The complete
64-bit namespace remains in the coordinate word. No namespace is shortened,
no font identity is reconstructed from a bare slot, and foreign or stale
identities remain distinguishable after rollback and candidate forks.

The coordinate field is nonzero, retaining the definition reference's existing
invariant without validating it again on each macro read. Scalar commands with no flags use the nonzero sentinel one; other scalar
flags remain complete in that same field. The definition reference remains
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

Expansion tests the actual native key and reads a primitive's real selector
from that key. Character scanners project the retained character/category
payload. Terminal dispatch uses the same key without an optional wrapper.
Read-only terminal entry borrows actual host facts; a real character flush
borrows the mutable execution owners.

The implementation must preserve the complete native suite, the existing
ordered canonical comparison and full checkpoint/group restoration. Matched
instruction and cycle measurements diagnose the resulting reader and macro
paths. If the replacement regresses, investigate its generated loop and fix
the representation or its consumers; no numerical acceptance threshold is
substituted for completing the redesign.

## Macro execution keys

An immutable definition determines one of three native execution keys:
parameterless empty, parameterless body, or matching parameter text. Literal
prefixes require the matching key even when they capture no numbered arguments.
The key occupies the existing command discriminator; flags and the complete
definition coordinate remain in their existing fields. There is no additional
kind column, activation request, or unclassified live macro key.

Macro keys use the command word's top bit, followed by the actual execution
shape. Root bookkeeping and suppression can test that bit directly rather than
checking three keys. Ordinary native keys and the unknown-scalar sentinel stay
below it. Unsupported raw primitive selectors exceeding that native range retain
their complete canonical scalar in the coordinate word; no selector or flags
are truncated. Valid static primitives remain directly indexed.

Publication selects the key from the final immutable words and parameter-text
length. The existing move-only `PublishedDefinition` owns that complete native
meaning instead of a bare reference; its reference projection still names the
same counted root. Assigning different flags preserves its execution key.
Formats keep their canonical flags and definition index. Loading reconstructs
the native key from the actual immutable format row. Aliases, native group saves,
checkpoint preimages and accepted alternates copy the complete native meaning.
Restore does not rebuild classification or admit a body.

Invocation dispatches on the native key. Ordinary empty macros acquire no body;
ordinary nonempty simple macros acquire only the body their input frame owns.
Matching and contextual tracing acquire the immutable payload for their actual
grammar. The invocation-time `AdmittedMacroDefinition` product and its forwarding
APIs are deleted. Scalar diagnostics, detached memo values and format output
continue to project the canonical macro meaning and flags on demand.

Quiet empty macros remain inside the retained reader span when
the consumed word leaves that actual frame live. A depleted stored row must
still retire before replacement handoff, including replay completion fences.
Tracing, observation, protected scanning and outer-token recovery retain their
actual entry boundaries. Fuel and input high-water accounting belong at the
actual expansion; avoiding a body or frame must not omit those charges.
