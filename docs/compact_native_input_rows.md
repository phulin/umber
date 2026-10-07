# Compact native input rows

The input stack owns one oldest-to-current vector of actual rows. The common
frame needs identity, relative delivery position, limit and source context.
The storage owner determines whether that row is a source, replacement,
argument, immutable replay or transient list. Storing that classification again
in every frame makes ordinary macro admission and retirement carry unrelated
replay metadata.

Keep the common frame at those four actual facts. A replacement borrows its
immutable definition words and selects the replacement with that owner’s actual
parameter extent, with position zero at the first replacement word.
An argument likewise indexes its actual argument span. Only transient lists need
an offset into the shared word vector; that offset belongs to their storage
variant, alongside their real allocation treatment. Immutable and transient
replays also own their genuine list kind and delivery/retirement flags there.
Transient offsets and relative frame cursors remain bounded `u32` coordinates.
Admission validates the actual vector coordinate with one direct upper-bound
check. On native 64-bit targets their sum cannot overflow, so resident selection
needs no additional arithmetic-overflow branch. No conversion-result enum is
constructed merely to establish the offset.
Sources, replacements and arguments derive their fixed kind and treatment from
the actual storage variant rather than copying those fields into a frame. The
body/argument capture grammar needs no replay-suppression guard; replay lists do
not enter it. Resident selection checks its actual range once, without first
constructing a separately checked replacement slice.

Argument capture selects its actual packed/traced span once. Inside a group,
ordinary space/letter/other spellings cannot complete the argument, match its
outer delimiter, or change brace depth. Their inner loop reads no writer or
delimiter bookkeeping. A special spelling ends that span and passes its one
local category classification to the full admission grammar. That grammar still
checks actual completion/delimiters and accounts for brace or first-word changes;
the next span reads the writer's real depth again. No second traversal, copied
configuration, physical cursor, or retained classification is introduced.

Source context stays in the common frame. It records semantic attribution that
can survive retirement or replacement of a physical source. Removing it or
reconstructing it from a source-stack walk would change that contract. No new
current-source cache, ancestry index or input owner is needed.

Admission constructs the final input row with its actual list offset, length,
kind and flags. Returning a storage union only to match it again for length and
copy it into a second aggregate leaves overlapping enum copies in generated
single-token backup code. Each real source knows its length while admitting its
words and initializes the final row directly; a one-word backup has length one.
There is no intermediate storage-union result or subsequent classification.
Macro and argument admission constructs only the common cursor
and its real payload. The previous frame start/kind/flags columns, packing
constructor and absolute-offset API are deleted; callers use actual geometry.

Aftergroup linking extends and reverses the real transient list using its own
offset. Retirement truncates that same span and drops the same payload.
V-template hold/awaiting state belongs to its replay owner. Source lexer steps
continue to advance the common relative position and limit. Full capture copies
the actual frame and replay owner in the existing row order; it gains no new
journal, delayed cursor write or continuation object.

On the native 64-bit target this permits a 24-byte common frame and a 64-byte
input row, replacing 32 and 72 bytes. Confirm the actual layout and generated
macro read/push/retirement paths, preserve canonical observations and exact fuel,
and measure frozen binaries on macros, scanners and the original book. A
smaller record by itself does not establish a speedup.
