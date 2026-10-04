# Native dispatch opcode

The current 16-byte meaning stores a scalar meaning-family tag and operand in
its command word. Expanded delivery classifies that family; main control then
translates it again into the compact dispatch code. The input and mode owner
changes remove repeated stack-row selection, but this repeated interpretation
of the same actual command remains in ordinary delivery.

Store the actual dispatch opcode in the command word instead of the former
meaning-family tag. The existing disjoint control-code ranges already identify
literal categories, expandable primitives, unexpandable primitives and value
families. A 16-bit opcode and 48-bit operand fit the existing command word.
The second word remains the complete opaque coordinate. There is no additional
selector cache, decoded kind enum, rooted command packet or third word.

Ordinary terminal dispatch reads the stored opcode directly. Expansion tests
the actual expandable range and executes its primitive; macro activation reads
the same native macro opcode, flags and complete definition reference.
Undefined and end-template handling retain their real semantic branches.
Literal character opcodes retain the category, and their operand retains the
character. Primitive opcodes retain the selector themselves rather than
duplicating it in an operand. Value families retain their complete operands.

Macro flags use the command word's operand space. Fonts retain their complete
32-bit allocation generation and 15-bit slot there, with the full 64-bit
namespace in the coordinate. Definition coordinates remain unchanged. Dense
meaning cells, group saves and independent full-checkpoint preimages continue
to own the same definition roots. Reading an opcode acquires no root and adds
no checkpoint or group bookkeeping.

The scalar constructor's complete existing 64-bit value domain must remain
injective. Scalars with reserved macro/font tags, noncanonical flag bits or
primitive operands outside the native selector range use one reserved opcode
and keep their full scalar value in the coordinate. Zero needs no escape.
This is a value-domain encoding rule; no old native-row representation,
format importer or alternate interpreter is retained.

The semantic scalar-word projection reconstructs the requested value where a
real semantic codec or consumer requires it. Ordinary expansion and main
control must use native opcode/operand projections instead of reconstructing
that word merely to classify it again. Preserve arbitrary scalar round trips,
literal/active/frozen spelling, complete foreign/stale font and definition
rejection, group restoration, admitted full checkpoints and demanded tracing.

Validate the complete native suite, authoritative quality gates and exhaustive
independent canonical comparison. Measure normal terminals, real macro frame
activation and literal-heavy numeric scans separately. The intended result is
a simpler dispatch representation and faster processing; no numerical gate
replaces either obligation.
