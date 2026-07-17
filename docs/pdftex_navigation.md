# pdfTeX destinations, outlines, and article threads

Status: implementation contract for the pdfTeX 1.40.27 navigation slice.

## Upstream boundary and oracle method

Behavior is pinned to `pdftex.web` at TeX Live source commit
[`1664cf0ab3f6ce3b80db649bc6723f54ab12016c`](https://github.com/TeX-Live/texlive-source/blob/1664cf0ab3f6ce3b80db649bc6723f54ab12016c/texk/web2c/pdftexdir/pdftex.web),
the same boundary as `pdftex_primitives.md`. The source checksum is
`5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04`.
The executable oracle is pdfTeX
3.141592653-2.6-1.40.27 from TeX Live 2025, run in INITEX mode with
`pdftex -ini -interaction=nonstopmode`.

The owning source sections and routines are:

- primitive registration and scanning at `pdf_outline_code`,
  `pdf_dest_node`, `pdf_thread_node`, `pdf_start_thread_node`, and
  `pdf_end_thread_node`;
- `scan_action`, `scan_alt_rule`, `scan_thread_id`, and `warn_dest_dup` for
  exact grammar and scanner diagnostics;
- `do_dest`, `do_thread`, `append_thread`, `append_bead`, and `end_thread` for
  page traversal, rectangles, ownership, and running-thread state;
- `open_subentries`, `out_thread`, `pdf_fix_dest`, and `pdf_fix_thread` for
  document-final hierarchy and missing-target behavior; and
- `Output name tree`, `Output outlines`, `Output article threads`, and
  `Output the catalog object` for the final PDF graph.

There is no `\pdfthreadname` primitive in pdfTeX 1.40.27 or in the pinned
158-name inventory. A thread is identified by the `name <general text>` arm of
`\pdfthread` or `\pdfstartthread`. When no bead supplies `attr`, pdfTeX derives
the thread dictionary's default `/I << /Title (...) >>` from that identifier.

## Ownership and reserved codecs

| Surface                                                              | Contract                                                                                  | Implementation issue                                  |
| -------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ----------------------------------------------------- |
| `\pdfdest`, `\pdfdestmargin`, `\pdfsuppresswarningdupdest`           | node scanner, placement, duplicate policy, destinations and name tree                     | `umber2-kbz0.16.3`, with warning threshold in `.16.1` |
| `\pdfoutline`                                                        | immediate document state, shared action scanner, hierarchy and final outline dictionaries | `umber2-kbz0.16.4`                                    |
| `\pdfthread`, `\pdfstartthread`, `\pdfendthread`, `\pdfthreadmargin` | bead nodes, page-local running state, thread array and circular bead graph                | `umber2-kbz0.16.5`                                    |
| all structures, diagnostics, replay, and render parity               | hermetic INITEX and PDF corpus                                                            | `umber2-kbz0.16.6`                                    |

The five new unexpandable stored meanings are reserved consecutively at
260 through 264 in source-inventory order: `pdfoutline`, `pdfdest`,
`pdfthread`, `pdfstartthread`, and `pdfendthread`. This slice adds no internal
integer or expandable meaning; 19 and 86 remain the next free operands for a
later branch. Existing parameter-bank cells own `pdfdestmargin`,
`pdfthreadmargin`, and `pdfsuppresswarningdupdest`.

Committed artifact tags 18 through 21 are reserved for destination,
one-shot thread bead, running-thread start, and running-thread end effects.
The image-ingestion branch owns tag 17 and schema v18, so navigation first
writes schema v19 while continuing to decode every accepted v1--v18 artifact.
Codec tests must enumerate all meanings and tags, reject unknown values, and
prove that no accepted legacy byte changes interpretation.

`tex-state` owns checkpointed document navigation records, identifier maps,
outline construction state, and page-local pending effects. `tex-exec` owns
the exact scanners and nodes. Page/form traversal owns final coordinates and
collision timing. `tex-out` owns only detached validated records and final
typed lowering. All generated dictionaries, arrays, actions, strings,
destinations, name-tree nodes, outlines, threads, beads, and references pass
through the vendored `pdf_writer`; backend code must not concatenate PDF
syntax.

## Shared identifiers and object allocation

Destination and thread identifiers have two disjoint forms:

```text
identifier := num <positive integer> | name <expanded general text>
```

Numbers greater than TeX's `max_halfword` are rejected as `number too big`.
Name tokens are expanded at scan time like `\special`, then retained as bytes.
Numeric and byte-name identities never alias. A name's PDF string escaping is
part of its source identity. For ordering, pdfTeX compares the retained bytes
while decoding backslash literal-string escapes (including octal); it does not
canonicalize hex-string spelling or strip user-supplied delimiters.

Navigation uses the canonical shared indirect-object ledger, but these system
allocations do not change `\pdflastobj`, which reports only the most recent
user raw-object operation. Allocation is source ordered and checkpointed:

- an outline immediately reserves an action object, an outline-item object,
  and an indirect title-string object, in that order;
- a destination reserves its identity object when first needed during page
  traversal or earlier action lowering; a later destination fills that same
  object;
- a thread identity is reserved when first referenced or when its first bead
  is traversed; every bead then reserves one bead object, and each page also
  reserves one indirect rectangle object per bead;
- outline root, name-tree levels, thread-reference array, page tree, catalog,
  and any fixed missing target are finalizer allocations in deterministic
  source-compatible order.

Rollback before commit restores all maps, counters, hierarchy links, pending
effects, and ledger reservations. Replaying the same accepted prefix must
reproduce object ids, artifact bytes, semantic hashes, diagnostics, and final
PDF bytes.

## Destinations

The exact grammar is:

```text
\pdfdest [struct <positive integer>]
         (num <positive integer> | name <expanded general text>)
         (xyz [zoom <integer>] | fitbh | fitbv | fitb |
          fith | fitv | fitr [width <dimen>] [height <dimen>]
                              [depth <dimen>] | fit)
```

Longer prefix-sharing keywords are tested first. The optional space after the
destination kind is consumed before `fitr` scans its rule specification.
Unspecified `fitr` dimensions are running dimensions. `zoom` accepts any
integer no greater than `max_halfword`, including zero and negative values;
pdfTeX emits it as thousandths with its source formatting. Omitting `zoom`
emits PDF `null`, not numeric zero.

Without `struct`, the destination belongs to the page on which its node is
finally traversed. `xyz`, `fith`/`fitbh`, and `fitv`/`fitbv` capture respectively
the current point, y, or x. `fit` and `fitb` need no coordinates. `fitr` uses
the explicit or containing-box rectangle. `\pdfdestmargin` expands all four
`fitr` edges at traversal time. For other coordinate-bearing kinds it affects
the transformed bounding calculation only while a page matrix is active.
Thus the live grouped margin at shipout, not its scan-time value, is observed.

`struct <object>` replaces the page reference in the destination array and
stores the destination in the separate structure-destination identity map.
Structure destinations are not inserted into the catalog name tree. A missing
referenced structure destination warns but has no synthetic fallback.

Destinations are forbidden while traversing a Form XObject and report
`destinations cannot be inside an XForm`. Nodes encountered in leaders are
silently ignored. Otherwise the first traversed node owns its identity and
its page; later duplicates are ignored. Two unshipped duplicates therefore
collide during traversal, while a duplicate scanned after its predecessor has
already shipped is removed at scan time. The diagnostic is:

```text
pdfTeX warning (ext4): destination with the same identifier
(name{<bytes>}|num<n>) has been already used, duplicate ignored
```

`\pdfsuppresswarningdupdest > 0` suppresses the warning only. Zero and every
negative value warn. The value is read when the collision is detected, so
fixtures must distinguish scan-time from shipout-time collisions and change
the parameter between construction and traversal.

Named page destinations are emitted as indirect dictionaries containing
`/D [<page> 0 R <kind> ...]`. Numeric destinations are indirect arrays and
are found only through numeric actions. Named destinations enter a byte-string
name tree sorted by decoded PDF string value. Each leaf or intermediate node
has at most six children and `/Limits`; leaves use `/Names`, parents use
`/Kids`, and the catalog `/Names` dictionary points to the root as `/Dests`.

An action may reserve a destination that is never defined. At finalization
pdfTeX warns that it `has been referenced but does not exist, replaced by a
fixed one` and writes `[<first existing page> 0 R /Fit]` into the reserved
object. Named fixed destinations remain in the name tree.

## Outlines

The exact grammar is:

```text
\pdfoutline [attr <expanded general text>]
            <action>
            [count <integer>]
            <expanded general text title>
```

The shared action grammar is the same one used by links:

```text
user <expanded general text>
goto   [file <text>] [struct (name <text> | num <positive> | <text with file>)]
       (page <positive> <view text> | name <text> | num <positive>)
       [newwindow | nonewwindow]
thread [file <text>] (name <text> | num <positive>)
```

The source scanner enforces action-specific combinations and exact messages:
missing action or identifier types, nonpositive page/number/structure ids,
`struct` on a non-GoTo action, local `goto num` combined with `file`, and
window flags without a remote GoTo are recoverable `ext1` errors. Implementors
must reuse the accepted typed action model and scanner rather than define an
outline-only dialect.

The title and attribute are expanded immediately. The title is PDF string
syntax, not an inferred Unicode scalar sequence: an empty value becomes `()`,
a value already delimited by `(...)` is emitted verbatim, a syntactically valid
even-length `<hex>` value is emitted verbatim, and every other byte sequence is
wrapped in parentheses without additional escaping. pdfTeX stores that value
in its own indirect object. The implementation needs a validated typed
PDF-string-syntax value so this compatibility behavior cannot become an
unbounded raw-object escape hatch. Attributes are injected after standard item
keys; the typed writer boundary must expose supported outline appearance keys
or a validated typed compatibility dictionary, never raw backend framing.

`count = 0` makes an item a leaf. A nonzero count starts a child level whose
extent is exactly `abs(count)` following sibling entries. Positive means open;
negative means closed. Once that many children have been scanned, construction
ascends through every completed ancestor. At finalization pdfTeX derives
`Parent`, `Prev`, `Next`, `First`, `Last`, and signed descendant `Count` links.
The outline root's positive `/Count` is the number of currently visible
entries, not the total node count. No page owns an outline; it is immediate
checkpointed document state and may refer to a page or destination allocated
later.

Each item points indirectly to both its title and action. The final catalog
contains `/Outlines` only when at least one item exists. Document ordering and
allocation must remain deterministic even though pdfTeX's internal object-type
list is reverse-creation ordered.

## Article threads and beads

The scanners are:

```text
\pdfthread      [width <dimen>] [height <dimen>] [depth <dimen>]
                [attr <expanded general text>] <identifier>
\pdfstartthread [width <dimen>] [height <dimen>] [depth <dimen>]
                [attr <expanded general text>] <identifier>
\pdfendthread
```

The three dimensions may occur repeatedly and in any order; the last value
wins. Missing dimensions are running dimensions resolved from the containing
box. `attr` follows all dimensions. `\pdfthread` creates one bead and is legal
in horizontal or vertical lists. A node in leaders is ignored. A thread node
inside a Form XObject reports `threads cannot be inside an XForm`.

Running `\pdfstartthread` and `\pdfendthread` are vertical-list operations.
Either node found in an hlist reports respectively
`\pdfstartthread ended up in hlist` or `\pdfendthread ended up in hlist`.
Start captures identifier, dimensions, and current output nesting level. It
creates the first bead and causes eligible following vboxes at that same level
to append automatic beads. End must occur at the same output nesting level or
reports `\pdfendthread ended up in different nesting level than
\pdfstartthread`. For a running depth, end adjusts the last bead's bottom edge
to the current vertical position plus the live margin.

Running state is reset at every page shipout, so a start/end pair cannot span
pages. It belongs to final traversal rather than TeX grouping. The implementation
must diagnose unmatched/misnested lifecycle effects without leaking state or
objects from a failed page commit.

Every bead rectangle is expanded on all sides by the live
`\pdfthreadmargin`. Beads belong to the page on which they are traversed; the
page dictionary lists them in `/B`. Each thread is a document-level dictionary
whose `/F` points to its first bead. Beads form a circular doubly linked list:
the first has `/T <thread>`, and every bead has `/V`, `/N`, `/P`, and an
indirect `/R` rectangle. The catalog `/Threads` value points to an indirect
array of thread references.

An `attr` belongs to its bead during collection. When the thread dictionary is
written, the last nonempty attribute encountered around the bead ring wins and
is used as the thread dictionary body. With no attribute, pdfTeX emits
`/I << /Title (<identifier>) >>`; numeric identifiers use decimal digits and
named identifiers use their retained bytes. This is the source meaning of a
thread "name"—there is no separate primitive.

An action may reserve a thread never populated by a bead. Finalization warns
that the named or numeric destination `has been referenced but does not exist,
replaced by a fixed one`, then creates a synthetic full-page bead, thread
dictionary, and links using the first page and configured PDF page dimensions.

## `pdf_writer` extensions required by parity

The vendored crate already has typed outline, destination, action, and generic
name-tree writers, but the following upstream-compatible typed APIs are needed
before lowering this slice:

- `Destination::xyz_nullable(left, top, Option<zoom>)` so absent zoom writes a
  PDF null rather than the existing method's numeric zero;
- string-key `/Limits` for `NameTree`, a typed named-destination dictionary
  whose `/D` value is a destination array, and a validated PDF-string-syntax
  value for pdfTeX's retained literal/hex/bare forms;
- indirect action creation plus typed outline setters for indirect `/Title`
  and `/A`, without forcing the optional `/Type /Action` key that pdfTeX omits;
- catalog `/Threads`, page `/B`, thread dictionary, bead dictionary, and
  thread-array writers; and
- typed title/default-info setters that preserve pdfTeX's exact string-syntax
  compatibility rules.

These extensions belong in the pinned `pdf-writer` fork with focused byte tests. Generic
`Dict::pair` calls or opaque fragments in `tex-out` are not an acceptable
substitute for a missing navigation API.

## Fixture matrix

All live-reference regeneration goes through `scripts/regen-fixtures.sh`.
The `tex_exec` rows run pdfTeX 1.40.27 in INITEX mode and normalize diagnostics;
the `pdf` rows pin reference PDF, normalized graph, exact Umber bytes, Poppler
render, digest chain, and retained-session replay.

| Case                              | Required observations                                                                                                                                                                                                           |
| --------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pdf_navigation_dest_scan`        | name/num and struct identities; all eight kinds; absent/present, zero, and negative zoom; running/explicit `fitr`; longest-keyword order; nonpositive, overflow, missing-id, and missing-kind diagnostics                       |
| `pdf_navigation_dest_lifecycle`   | page ownership, leaders, Form rejection, matrix/no-matrix coordinates, live positive/zero/negative margin, pre-ship and post-ship duplicates, live suppression `-1/0/1`, missing regular and structure destinations             |
| `pdf_navigation_outline_scan`     | all shared action variants and invalid combinations; count absent/zero/positive/negative; byte, escaped, and UTF-8-input title bytes; attr ordering; three-object allocation without changing `pdflastobj`                      |
| `pdf_navigation_outline_tree`     | multilevel open/closed tree, automatic ascent, root visible count, all sibling/parent/child links, forward page/destination references, empty omission, snapshot replay                                                         |
| `pdf_navigation_thread_scan`      | named/numeric ids; dimension permutations and running values; attr placement/expansion; missing/nonpositive/overflow id diagnostics; DVI-mode rejection                                                                         |
| `pdf_navigation_thread_lifecycle` | one-shot h/v beads, start/end in a vlist, hlist errors, different-level error, leaders, Form rejection, page reset, automatic vbox beads, live positive/zero/negative margins, failed-page rollback                             |
| `pdf_navigation_thread_graph`     | page `/B`, catalog `/Threads`, circular one/many-bead links, first-only `/T`, page and rectangle ownership, last nonempty attr wins, default named/numeric titles, missing-thread fixed fallback                                |
| `navigation_structures`           | more than six deliberately unsorted escaped names, balanced name-tree levels and limits, mixed outline/thread/destination graph, multiple pages, exact deterministic ids, normalized dictionaries/arrays, unchanged page pixels |

The focused completion sequence is the new scanner/state tests, the committed
navigation corpus, `cargo test --tests`, and `scripts/check.sh`.
The full epic gate additionally runs `scripts/check-and-test.sh`, the PDF
regeneration validation path, DVI byte parity, and the native/WASM gates.
