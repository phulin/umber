# pdfTeX graphics state, positions, timer, and random state

Status: implementation contract for the pdfTeX 1.40.27 graphics-state slice.

## Upstream boundary and oracle method

Behavior is pinned to `pdftex.web` and `utils.c` at TeX Live source commit
[`1664cf0ab3f6ce3b80db649bc6723f54ab12016c`](https://github.com/TeX-Live/texlive-source/tree/1664cf0ab3f6ce3b80db649bc6723f54ab12016c/texk/web2c/pdftexdir),
the same boundary as `pdftex_primitives.md`. The source files have these
relevant ownership boundaries:

- `pdftex.web` registers and scans the primitives, creates whatsits, expands
  literal text, lowers nodes during page or form traversal, computes saved
  positions, and defines timer and random-seed enquiries.
- `utils.c` owns color-stack page/form state, matrix and save/restore stacks,
  matrix validation, and shipout-balance diagnostics.
- the executable oracle is pdfTeX 3.141592653-2.6-1.40.27 from TeX Live 2025,
  run with `pdftex -ini -interaction=nonstopmode`.

The source checksum for `pdftex.web` is
`5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04`.
`\pdfsnaptorefpoint` is not a pdfTeX primitive and does not occur in the pinned
158-name inventory. The source-level snapping family is `\pdfsnaprefpoint`,
`\pdfsnapy`, and `\pdfsnapycomp`; all three are retained below because this is
a source-inventory parity project rather than a manual-only subset.

## Primitive ownership checklist

| Primitive | Contract | Implementation owner |
| --- | --- | --- |
| `\pdfliteral` | immediate/shipout expansion, origin/page/direct lowering | `umber2-kbz0.12.2` |
| `\pdfsetmatrix` | four-number scanner and tracked transform | `umber2-kbz0.12.2` |
| `\pdfsave`, `\pdfrestore` | balanced transform stack and `q`/`Q` | `umber2-kbz0.12.2` |
| `\pdfcolorstackinit`, `\pdfcolorstack` | allocation, page/form stacks, four operations | `umber2-kbz0.12.3` |
| `\pdfsavepos`, `\pdflastxpos`, `\pdflastypos` | shipout coordinates in sp | `umber2-kbz0.12.4` |
| `\pdfsnaprefpoint`, `\pdfsnapy`, `\pdfsnapycomp` | vertical-grid adjustment | `umber2-kbz0.12.4` |
| `\pdfxform`, `\pdfrefxform`, `\pdflastxform`, `\pdfxformname` | captured boxes and reusable Form XObjects | `umber2-kbz0.14.4` |
| `\pdfresettimer`, `\pdfelapsedtime` | deterministic monotonic 16.16 timer | `umber2-kbz0.9.5`, integrated by `umber2-kbz0.12.5` |
| `\pdfsetrandomseed`, `\pdfrandomseed` | global seed and exact RNG reset | `umber2-kbz0.9.5`, integrated by `umber2-kbz0.12.5` |

The stored-meaning codec keeps the previously accepted object/dictionary
operands unchanged: `\pdfobj` through `\pdftrailerid` occupy unexpandable
operands 231 through 237 and `\pdflastobj` occupies internal-integer operand
13. The graphics family is appended at 238 through 246, while
`\pdflastxpos` and `\pdflastypos` use 14 and 15. Checkpoint schema 22 composes
the canonical object ledger with color-stack and saved-position state. An
exhaustive codec test requires every stored meaning through these endpoints to
decode bijectively, preventing later branch integrations from reusing an
accepted operand.

Tagged-spacing operations retain operands 247 through 250. Form creation and
reference were therefore appended at 251 and 252; `\pdflastxform` uses
internal-integer operand 16 and `\pdfxformname` uses expandable operand 84.

## Form XObjects

`\pdfxform` reserves its canonical Form XObject and resource-dictionary object
identities before it scans optional `attr`, optional `resources`, and a
box-register number. Consequently the observable form handles advance through
the shared ledger as 1, 3, 5, ... when no other objects intervene, while
`\pdfxformname` reports the independent sequential resource name 1, 2, 3, ... .
The typed backend may inline the resource dictionary, but its reserved ledger
identity remains observable and checkpointed. A
nonvoid box is consumed with ordinary same-level assignment semantics and its
dimensions, immutable node root, expanded attributes, and expanded resources
are retained checkpointably. `\pdfrefxform` validates the object and appends a
dimensioned reusable node; horizontal traversal advances by the form width,
while vertical traversal advances by height, places the form at its baseline,
then advances by depth. Normal forms are traversed lazily when their owning
page or form is finally shipped; `\immediate\pdfxform` traverses at once.

Each first traversal produces a schema-v16 detached artifact. Recursive form
references remain typed artifact effects, so nested rules, text and physical
glyph codes, graphics operations, font resources, and tagged spacing follow
the same normalized traversal as pages. Form origins are always zero. Saved
positions publish `(cur_h, height + depth - cur_v)` and form snapping begins
from its own zero reference, without importing or replacing the page grid.
Color stacks use their independent form projection; matching pdfTeX 1.40.27,
that projection persists between successful form traversals while never
mutating the page projection. A failed transactional traversal publishes no
artifact or saved-position suffix and rolls its color changes back.

Final assembly emits only immediate or transitively referenced forms. Page and
form resource dictionaries map `/Fm<n>` to the canonical object, nested forms
are deduplicated, and cycles are rejected. `/Type /XObject`, `/Subtype /Form`,
`/BBox`, identity `/Matrix`, `/FormType 1`, attributes, resources, streams, and
`Do` placement are all written through typed APIs in the vendored
`pdf_writer`; no backend-owned raw PDF framing is permitted.

The committed `pdf/form_xobjects` corpus pins decoded Form dictionaries and
streams, nested h/v/math placement, reuse, attributes/resources, exact Umber
bytes, and Poppler raster parity. The `tex_exec/pdf_form_state` and
`pdf_form_diagnostics` INITEX oracles pin allocation, enquiries, box
consumption, saved positions, snapping, lazy publication, and the void-box
diagnostic; the hermetic PDF integration test replays the committed source from
a retained checkpoint and requires identical artifacts, coordinates, bytes,
and state hash.

## Literals

The grammar is:

```text
\pdfliteral [shipout] [direct|page] <general text>
```

The primitive is legal in vertical, horizontal, and math mode, but requires
positive `\pdfoutput`; use in DVI mode is a fatal pdfTeX error. It appends a
zero-size whatsit to the current list. Without `shipout`, `<general text>` is
balanced and fully expanded while the node is built, like `\special`. With
`shipout`, the unexpanded token list is retained and expanded when the node is
traversed, like a non-immediate `\write`. The `shipout` keyword must precede
the mode keyword.

Lowering preserves list order and writes one newline after the supplied byte
string. The modes have distinct text and coordinate behavior:

- default (`set_origin`) closes an open text string and `BT`/`ET` section,
  then emits a translation from the previous PDF origin to the current TeX
  position before the literal;
- `page` closes text but does not translate, so the literal uses the page's
  lower-left PDF coordinate system;
- `direct` closes only an open `TJ` string and may therefore insert bytes
  inside `BT`/`ET`; it neither closes text nor translates.

Umber stores typed literal effects in the executable node/page-artifact
timeline. Literal payload bytes may be opaque because pdfTeX intentionally
does not parse them, but page-stream framing, coordinates, and every generated
operator remain typed. Final content serialization must use the vendored
`pdf_writer` content API. If that API cannot represent the required opaque
operation or text-state transition, extend the vendored crate with a typed
upstream-compatible API; do not concatenate raw PDF syntax in `tex-out`.

## Transform stack

`\pdfsetmatrix` scans one immediately expanded balanced general-text argument.
At page shipout, the byte string must contain exactly four C `double` values
separated by arbitrary whitespace and no fifth non-whitespace item. Invalid
text fails during shipout with
`pdfTeX error (\pdfsetmatrix): Unrecognized format.` The four values are
emitted as `a b c d 0 0 cm` at the current TeX point. pdfTeX also composes a
tracked six-value matrix about that point for annotation geometry. During form
shipout it emits the supplied values but does not update the page annotation
matrix stack.

`\pdfsave` and `\pdfrestore` take no arguments, are legal in all TeX modes,
require PDF output, and append zero-size whatsits. At traversal, save records
the current horizontal/vertical position and tracked matrix depth, then emits
`q` after moving the PDF origin to that point. Restore emits `Q` after the same
origin synchronization and restores the tracked page matrix depth. The
following diagnostics are shipout-time behavior, not scanner behavior:

- restore with no unmatched save warns
  `\pdfrestore: missing \pdfsave` and still emits `Q`;
- restoring at a different TeX position warns
  `Misplaced \pdfrestore by (<x>sp, <y>sp)` and still emits `Q`;
- any saves left at the end of a page or form are fatal, reporting the count
  and whether page or form shipout was active.

The implementation must validate the same-position and nesting invariants on
the final traversal, not by TeX group level. Groups do not scope PDF graphics
state; the manual's group-level advice is a user discipline rather than an
additional source check. Save/restore effects and tracked matrices must be
part of detached artifact identity and deterministic replay.

## Color stacks

`\pdfcolorstackinit` is expandable and has grammar:

```text
\pdfcolorstackinit [page] [direct|page] <general text>
```

The first optional `page` controls whether the current value is restored at
each page start. The second optional keyword selects the same literal mode as
`\pdfliteral`; no second keyword means origin-relative mode. Thus `page page`
is meaningful. The balanced initial text is expanded immediately. Allocation
is global and ungrouped. Stack 0 exists lazily with initial/current value
`0 g 0 G`, direct mode, and page-start restoration enabled. User allocations
start at 1, are monotonic, and are limited to 32,768 total stacks; exhaustion
reports `Too many color stacks` and returns stack 0 after recovery.

`\pdfcolorstack <integer> <action>` requires positive `\pdfoutput` and appends
a traversal-time node. An unknown number recovers to stack 0 after `Unknown
color stack number <n>`; a negative number separately recovers after `Invalid
negative color stack number`. Missing or misspelled actions report `Color
stack action is missing` and append no node. The operations are:

- `set <general text>` replaces current without changing depth and emits it;
- `push <general text>` saves current, replaces it, and emits the replacement;
- `pop` removes the top saved value and emits the restored current value;
- `current` emits current without changing it.

Setter text is expanded when scanned. Mutation occurs only when the final
page/form list is traversed, so TeX grouping does not undo an operation. A pop
from an empty page stack warns `pop empty color page stack <n>`; a form pop
uses `form` in that diagnostic. It leaves the current value unchanged and
emits no bytes.

Each allocation has separate page and form current values and stacks. Page
state persists across page shipouts. When page-start restoration is enabled,
the current page value is emitted before page material except when it is the
default `0 g 0 G`; an empty value emits nothing. Form state also persists
across successive form shipouts and does not mutate page state. This last
detail is an observable 1.40.27 quirk: `utils.c` contains a form-initial-value
reset helper, but the release's shipout call and guard combination does not
execute that reset. A form that sets a stack to `FORM-ONE` therefore causes a
later form's `current` operation to emit `FORM-ONE`, not the allocation's
initial value. The configured literal mode applies to all values emitted by
that stack.

Umber's allocation and pending nodes participate in semantic hashes and
snapshots. Rollback discards an allocation/effect suffix so replay receives
the same stack numbers and traversal order. Committed page/form effects keep
the page and form projections separate. Opaque user color bytes cross the
artifact boundary as typed color-stack operations and are written only via a
typed vendored `pdf_writer` content escape hatch.

## Saved positions

`\pdfsavepos` takes no arguments, is legal in vertical, horizontal, and math
mode, and appends a zero-size whatsit. Unlike the other PDF graphics nodes it
is also supported when `\pdfoutput <= 0`. Its effect occurs only while a final
page or form list is traversed. `\pdflastxpos` and `\pdflastypos` are
read-only integer enquiries, initially zero and unchanged merely by building
or boxing a save-position node.

For PDF pages, the saved values are the current horizontal position and
`page_height - current_vertical_position`, measured in scaled points from the
media lower-left corner. For form traversal, y is
`form_height + form_depth - current_vertical_position`. In DVI traversal,
pdfTeX applies DVI's one-inch origin conversion: x is `cur_h + 4736286` and y
is `page_height - cur_v - 4736286`, also in sp. Values become observable only
after the containing shipout completes; a normal cross-run use therefore
writes them from a later node or a later run.

Umber records the marker in the list/page artifact and publishes the two
enquiry values through the shipout commit barrier. A failed shipout cannot
change them. Snapshot restore before a committed shipout restores the prior
pair; deterministic replay publishes the same coordinates. Position lowering
must use the committed page geometry and offsets, never live post-shipout
state.

## Snapping

All three snapping controls require PDF output and append zero-size whatsits.
They are source-level compatibility primitives absent from the current pdfTeX
manual:

- `\pdfsnaprefpoint` records the current output `cur_h` and `cur_v` as the
  grid reference when traversed. The values start at zero and persist until a
  later reference node.
- `\pdfsnapy <glue>` scans ordinary glue. A negative natural width is fatal.
  In a vertical list, traversal moves `cur_v` to a neighboring point on the
  grid `reference + k * width`. Backward movement is allowed only when below
  the glue shrink limit and forward movement only when below the stretch
  limit; infinite orders allow the corresponding direction without a finite
  bound. If neither is allowed there is no movement, and an exact tie moves
  forward. In a horizontal list the node is a no-op.
- `\pdfsnapycomp <integer>` silently clamps its ratio to `0..=1000`. In a
  vertical list it searches forward in the same containing list for the next
  `\pdfsnapy`, computes that node's final grid correction, applies the stated
  ratio (per thousand) at the compensation node, and leaves the remainder on
  the snap node. It is a no-op in a horizontal list or when no later snap node
  exists.

Snapping changes positioning only and emits no PDF operator. The three nodes,
reference state, compensation remainder, and resulting movements must be in
the checkpointed execution/artifact model so reuse cannot omit or duplicate a
grid adjustment. There is no `\pdfsnaptorefpoint` alias to install.

Umber encodes saved-position and snapping nodes as anchored `PageEffect`s. The
shared positioned traversal publishes the last raw save point and final grid
reference after a successful walk; the executor then converts the save point
to pdfTeX's PDF- or DVI-specific page coordinates. Pages containing snapping
controls compile their DVI plan from the committed artifact so fresh and
replayed output use the same same-list lookahead. No PDF syntax is emitted for
these effects; PDF serialization continues through the vendored `pdf_writer`
adapter.

## Timer and random state

These controls are already implemented by `umber2-kbz0.9.5`; the graphics
slice consumes that implementation rather than creating parallel state.

`\pdfresettimer` takes no argument and immediately replaces the global,
ungrouped monotonic baseline. `\pdfelapsedtime` is a read-only integer in
ticks of 1/65,536 second since that baseline. The reference quantizes its
system microsecond observation to 100-microsecond units and saturates at
`2^31-1` after 32,767 seconds. Umber uses the World-provided deterministic
monotonic clock; its baseline and observable state participate in snapshots
and semantic identity, while formats start a new World session.

`\pdfsetrandomseed <integer>` uses the ordinary integer scanner, silently
changes a negative value to its absolute value, globally stores it, and
reinitializes pdfTeX's 55-word random table. It is immediate and ungrouped.
`\pdfrandomseed` reports the stored seed; drawing uniform or normal deviates
advances the table but does not change this enquiry. Setting `-123` and `123`
therefore produces the same subsequent sequence and both report `123`.
Umber's seed and complete generator state are checkpointed so rollback and
resumed native/WASM execution reproduce the sequence exactly.

## Reproducible oracle cases

The implementation fixtures use the repository's pinned PDF regeneration
entry point, `scripts/regen-fixtures.sh --area pdf`, and normalize metadata as
defined in `pdf_backend.md`. The focused case matrix is:

| Case | Required observations |
| --- | --- |
| `graphics_literals` | default/page/direct placement, immediate and shipout expansion, h/v/math nodes, DVI rejection |
| `graphics_transform` | exact `q`, `Q`, and `a b c d 0 0 cm` order; malformed matrix; missing, misplaced, and unmatched saves; page/form isolation |
| `graphics_colorstack` | stack 0, allocation order, `page page`, direct mode, all four operations, persistent but separate page/form state, invalid IDs and underflow |
| `graphics_savepos` | initial zero, nested boxes/shifts, origins/page geometry, PDF/form/DVI coordinates, pre/post shipout and failed shipout |
| `graphics_snap` | reference, finite/infinite stretch/shrink, tie-forward, compensation ratios 0/500/1000, hlist no-op, negative glue |
| `pdf_utilities` | reset timer, saturation seam, signed seed normalization, enquiry stability, snapshot replay, native/WASM sources; owned by issue 9.5 |

A minimal INITEX oracle sets brace/comment catcodes, disables stream and object
compression, sets explicit page dimensions and origins, builds the relevant
nodes in `\hbox`/`\vbox`, and ships them. For example, a page containing
`\pdfsave\pdfsetmatrix{1 0 0 1}\pdfrestore` normalizes to ordered `q`,
`1 0 0 1 0 0 cm`, `Q`; `\pdfcolorstackinit page page{INIT}` emits `INIT`
before first-page material and again before second-page material. A
`\pdfsavepos` 10pt from the left on an explicit 100pt page with zero origins
publishes x `655360` and y `6553600`. These observations were reproduced with
the pinned TeX Live 2025 executable during this audit.

All ordinary tests remain hermetic. Fixture regeneration is the only step
that invokes pdfTeX; committed tests compare normalized content operators,
diagnostics, saved integers, checkpoint replay, and unchanged DVI artifacts.
