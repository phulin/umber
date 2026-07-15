# pdfTeX 1.40.27 Primitive Checklist

This document inventories the primitive control sequences added by pdfTeX on
top of TeX82 and the original e-TeX change file. Beads is the source of truth
for implementation progress; this document fixes the upstream boundary and
the completeness gate.

## Upstream boundary and method

The authoritative source is `texk/web2c/pdftexdir/pdftex.web` at official
TeX Live source commit
[`1664cf0ab3f6ce3b80db649bc6723f54ab12016c`](https://github.com/TeX-Live/texlive-source/blob/1664cf0ab3f6ce3b80db649bc6723f54ab12016c/texk/web2c/pdftexdir/pdftex.web).
That file declares pdfTeX 3.141592653-2.6-1.40.27 and has SHA-256
`5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04`.
The repository's local parity oracle and committed corpus metadata pin the
same engine version (TeX Live 2025).

The inventory takes the unique names registered by `primitive(...)` in that
file, then subtracts the unique names registered by the matching TeX82
`tex.web` and original e-TeX `etex.ch`. This is intentionally a source-level
engine inventory: it includes obsolete compatibility controls and internal
aliases that a manual organized by user-facing features can omit.

| Layer | Registered names | Exact names currently installed by Umber |
| --- | ---: | ---: |
| TeX82 prerequisite | 325 | 325 |
| Original e-TeX prerequisite | 66 | 66 |
| pdfTeX layer | 158 | 158 in pdfTeX mode |
| Total | 549 | 549 in pdfTeX mode |

The prerequisite count is nominal control-sequence coverage; its behavioral
gates remain the TeX and e-TeX corpora. `umber run --pdftex` and the native or
WASM session option `engine: "pdftex"` select this layer and its truthful
1.40.27 identity. All 158 names are registered in that mode; names whose
semantics remain in later checklist issues fail explicitly as unsupported
rather than behaving like `\relax`.

In addition to `\expanded` and `\ifincsname`, the parameter slices now
implements `\pdfoutput`, PDF major/minor version, stream/object compression,
decimal precision, page geometry/attributes, and image-configuration state.
The other parameter effects remain assigned to checklist issues 6--8; the
remaining exact names still fail explicitly until their issue is complete.
There are two intentional pre-existing visibility overlaps: e-TeX mode keeps
`\ifincsname`, and the supported LaTeX-DVI contract keeps `\expanded` (and
inherits `\ifincsname` through e-TeX). The source-set gate therefore requires
all 158 names in pdfTeX mode and isolates the other 156 from earlier modes,
without changing either established contract.
Umber also exposes the engine-neutral names `\creationdate`, `\filesize`,
`\shellescape`, and `\strcmp`; these are implementation reuse candidates for
the corresponding `\pdf...` aliases, not exact-name coverage.

## Source-derived checklist

The families and order below follow the registration blocks in the pinned
source. **Done** means the exact name and observable pdfTeX behavior have an
oracle-backed test. Every name in a missing row is missing.

| Family (source registration block) | Count | Status | Primitive names |
| --- | ---: | --- | --- |
| PDF token-list parameters | 4 | done | `\pdfpagesattr`, `\pdfpageattr`, `\pdfpageresources`, `\pdfpkmode` |
| PDF integer parameters | 38 | partial (output policy, metadata effects, and microtype/font configuration contract done) | `\pdfoutput`, `\pdfcompresslevel`, `\pdfobjcompresslevel`, `\pdfdecimaldigits`, `\pdfmovechars`, `\pdfimageresolution`, `\pdfpkresolution`, `\pdfuniqueresname`, `\pdfoptionpdfminorversion`, `\pdfoptionalwaysusepdfpagebox`, `\pdfoptionpdfinclusionerrorlevel`, `\pdfmajorversion`, `\pdfminorversion`, `\pdfforcepagebox`, `\pdfpagebox`, `\pdfinclusionerrorlevel`, `\pdfgamma`, `\pdfimagegamma`, `\pdfimagehicolor`, `\pdfimageapplygamma`, `\pdfadjustspacing`, `\pdfprotrudechars`, `\pdftracingfonts`, `\pdfadjustinterwordglue`, `\pdfprependkern`, `\pdfappendkern`, `\pdfgentounicode`, `\pdfdraftmode`, `\pdfinclusioncopyfonts`, `\pdfsuppresswarningdupdest`, `\pdfsuppresswarningdupmap`, `\pdfsuppresswarningpagegroup`, `\pdfinfoomitdate`, `\pdfsuppressptexinfo`, `\pdfomitcharset`, `\pdfomitinfodict`, `\pdfomitprocset`, `\pdfptexuseunderscore` |
| PDF dimension parameters | 13 | done | `\pdfhorigin`, `\pdfvorigin`, `\pdfpagewidth`, `\pdfpageheight`, `\pdflinkmargin`, `\pdfdestmargin`, `\pdfthreadmargin`, `\pdffirstlineheight`, `\pdflastlinedepth`, `\pdfeachlineheight`, `\pdfeachlinedepth`, `\pdfignoreddimen`, `\pdfpxdimen` |
| Font construction and primitive recovery | 3 | done | `\pdfprimitive`, `\letterspacefont`, `\pdfcopyfont` |
| Read-only integer enquiries | 14 | partial (9 done) | `\pdftexversion`, `\pdfelapsedtime`, `\pdfshellescape`, `\pdfrandomseed`, `\pdflastobj`, `\pdflastxform`, `\pdflastxpos`, `\pdflastypos`, `\pdfretval` (done); `\pdflastximage`, `\pdflastximagepages`, `\pdflastannot`, `\pdflastximagecolordepth`, `\pdflastlink` |
| Expandable conversions and enquiries | 27 | partial (26 done) | `\expanded`, `\pdftexrevision`, `\pdftexbanner`, `\pdffontsize`, `\pdffontname`, `\pdffontobjnum`, `\leftmarginkern`, `\rightmarginkern`, `\pdfescapestring`, `\pdfescapename`, `\pdfescapehex`, `\pdfunescapehex`, `\pdfcreationdate`, `\pdffilemoddate`, `\pdffilesize`, `\pdfmdfivesum`, `\pdffiledump`, `\pdfstrcmp`, `\pdfmatch`, `\pdflastmatch`, `\pdfuniformdeviate`, `\pdfnormaldeviate`, `\pdfinsertht`, `\pdfximagebbox`, `\pdfcolorstackinit`, `\pdfxformname` (done); `\pdfpageref` |
| Primitive-identity conditional | 1 | done | `\ifpdfprimitive` |
| Horizontal-mode normalization | 1 | done | `\quitvmode` |
| Character codes and ligature control | 10 | done | `\lpcode`, `\rpcode`, `\efcode`, `\tagcode`, `\knbscode`, `\stbscode`, `\shbscode`, `\knbccode`, `\knaccode`, `\pdfnoligatures` |
| PDF backend actions | 43 | partial (29 done) | `\pdffontexpand`, `\pdfincludechars`, `\pdffontattr`, `\pdfmapfile`, `\pdfmapline`, `\pdfglyphtounicode`, `\pdfnobuiltintounicode`, `\pdfresettimer`, `\pdfsetrandomseed`, `\pdfobj`, `\pdfrefobj`, `\pdfinfo`, `\pdfcatalog`, `\pdfnames`, `\pdftrailer`, `\pdftrailerid`, `\pdfliteral`, `\pdfcolorstack`, `\pdfsetmatrix`, `\pdfsave`, `\pdfrestore`, `\pdfsavepos`, `\pdfsnaprefpoint`, `\pdfsnapy`, `\pdfsnapycomp`, `\pdfxform`, `\pdfrefxform`, `\pdfximage`, `\pdfrefximage` (done); `\pdfannot`, `\pdfstartlink`, `\pdfendlink`, `\pdfoutline`, `\pdfdest`, `\pdfthread`, `\pdfstartthread`, `\pdfendthread`, `\pdfinterwordspaceon`, `\pdfinterwordspaceoff`, `\pdffakespace`, `\pdfrunninglinkoff`, `\pdfrunninglinkon`, `\pdfspacefont` |
| Compatibility error policy | 1 | done | `\ignoreprimitiveerror` |
| Late expansion conditionals | 3 | done | `\ifincsname`, `\ifpdfabsnum`, `\ifpdfabsdim` |

Counts in the table sum to 158. No primitive is complete merely because its
name is installed: assignments must group correctly, expandable results and
diagnostics must match the pinned oracle, and node/effect state must survive
checkpoint, restore, semantic hashing, and format serialization where
applicable.

The source-derived graphics-state, literal, color-stack, saved-position,
snapping, timer, and random contracts are fixed in
`pdftex_graphics_state.md`. In particular, the pinned source has
`\pdfsnaprefpoint`, `\pdfsnapy`, and `\pdfsnapycomp`; it has no
`\pdfsnaptorefpoint` primitive. Timer and random behavior is shared with the
completed utility implementation rather than duplicated in the PDF driver.

The parameter-state slice uses reserved cells in the existing typed integer,
dimension, and token-list banks. pdfTeX mode initializes the cells to the
pinned INITEX defaults; other engine modes leave them untouched. Of the three
legacy `\pdfoption...` spellings, `\pdfoptionpdfminorversion` shares the
`\pdfminorversion` cell. The pinned
source and INITEX oracle prove that `\pdfoptionalwaysusepdfpagebox` and
`\pdfoptionpdfinclusionerrorlevel` instead have independent compatibility
cells.
Assignments therefore inherit TeX grouping, `\global`, and `\globaldefs`
semantics from the common environment barrier, while snapshots, semantic
hashes, and format images include the values through the existing bank
machinery. Downstream consumers must not introduce shadow parameter state.

The completed output-policy consumer freezes normalized values at the first
committed shipout and uses `pdf_writer` for ordinary streams, object streams,
type-2 cross-reference entries, and final framing. The committed INITEX oracle
at `tests/corpus/tex_exec/pdf_output_policy` covers defaults, grouping, range
recovery, and diagnostics; focused hermetic tests cover the shared
minor-version alias, the two independent obsolete inclusion cells, fatal
post-write changes, PDF headers and object-compression levels, decimal
rounding, and unchanged DVI output.

The INITEX metadata-configuration oracle at
`tests/corpus/tex_exec/pdf_metadata_config` covers the nine issue-8 parameter
names, their zero defaults, signed assignments, and ordinary grouping. The
default Info dictionary uses the pinned job clock for `/CreationDate` and
`/ModDate`; the live final controls implement complete Info/date omission,
odd `\pdfsuppressptexinfo` suppression, and `PTEX_` selection for positive
`\pdfptexuseunderscore` or PDF 2. `\pdfomitprocset` is captured at shipout:
negative values emit the page resource entry, zero emits it only before PDF 2,
and positive values omit it. All dictionary and trailer bytes flow through the
canonical vendored `pdf_writer` adapter. The warning controls remain real
configuration state; their collision-point effects are tracked under
`umber2-kbz0.16.1` (destinations), `umber2-kbz0.17.1` (font maps), and
`umber2-kbz0.14.1` (included-PDF page groups), avoiding invented no-ops before
those producers exist.

The INITEX image-configuration oracle at
`tests/corpus/tex_exec/pdf_image_config` covers all 14 issue-6 names. Integer
assignment accepts the ordinary signed TeX range and groups normally;
consumers clamp gamma values to `0..=1000000`, image high-color,
apply-gamma, and inclusion-copy-fonts to `0..=1`, nonzero PK resolution to
`72..=8000`, and image resolution to `0..=65535`. Draft mode retains its raw
fixed value and is enabled when positive; changing it after output is written
is a fatal setup error. `\pdfuniqueresname` is enabled only by a positive
value. The two obsolete inclusion controls are not no-ops: image scanning
warns once and transfers a nonzero value to the corresponding current
control. External-image registration and lowering now consume those controls.
`\pdfimageresolution` supplies missing raster DPI, including pdfTeX's
zero-to-72-dpi fallback; `\pdfimageapplygamma`, `\pdfimagegamma`, and
`\pdfgamma` transform PNG samples, while `\pdfimagehicolor=0` reduces 16-bit
PNG color and mask samples to 8 bits. PDF-page inclusion applies explicit and
live page boxes, obsolete warning-and-global-transfer behavior, and the signed
PDF-version error policy. Positive `\pdfuniqueresname` produces stable
content-derived XObject names. Positive `\pdfdraftmode` completes the run but
leaves a requested PDF path untouched and emits pdfTeX's warning.
`\pdfinclusioncopyfonts=0` differs in upstream pdfTeX only when an included
embedded Type-1 font has a matching host font-map entry. Umber's detached
external-image boundary has no host font-map substitution candidate, so both
values copy the included resource graph for the unmatched case.

The INITEX microtype/font-configuration oracle at
`tests/corpus/tex_exec/pdf_font_config` covers the nine issue-7 parameter
names, zero defaults, signed assignments, grouping, and
`\pdftracingfonts` box diagnostics. `PdfFontConfiguration` is a typed live
projection of the canonical integer cells, not separately stored state. It
pins the source-level distinctions that adjustment and protrusion affect final
line material at values above zero but add line-breaking work only above one;
interword adjustment and prepend/append kerns use positive values. A zero PK
resolution selects the driver-provided DPI and the output consumer clamps the
result to `72..=8000`. ToUnicode generation is positive-only, while any
nonzero `\pdfomitcharset` omits the eligible subset Type-1 `/CharSet` entry.
The latter two contracts come from `writefont.c` at the same pinned TeX Live
source commit as `pdftex.web`.

Issue 7.2 now consumes that projection for the effective paragraph and font
dictionary behavior. The pinned INITEX `pdf_microtype_effects` oracle covers
positive and inactive negative prepend/append kerns, interword width/stretch/
shrink adjustment, protrusion margin kern placement, and expansion diagnostics.
The pinned PDF `embedded_subset_type1`, `embedded_subset_omit`, and
`embedded_subset_controls_negative` fixtures prove positive and nonpositive
ToUnicode generation and zero/nonzero CharSet omission against pdfTeX 1.40.27.
The font dictionaries and CMaps continue to serialize only through the
canonical `pdf_writer` adapter. Child issue 7.2.1 completes PK output with a
bounded, host-neutral decoder and checkpointed typed resource provider. The
driver resolves the frozen `\pdfpkresolution` and `\pdfpkmode`, selects exact
`name.<dpi>pk` resources, and emits Type3 fonts with resolution-dependent
matrices, widths, bounding boxes, and image-mask CharProcs. The vendored
`pdf_writer` content API owns the typed inline-image dictionary and all
`BI`/`ID`/`EI` framing. Real `cmr10` PK-only fixtures at 300 and 600 DPI match
pdfTeX 1.40.27 structure, extracted text, and rendered pixels. Focused tests
pin the zero sentinel to the host-provided driver DPI, clamp nonzero values to
`72..=8000`, and cover negative values through the same lower bound.

`\pdfinsertht` reads the page builder's live insertion record directly. It
returns `0pt` for a class with no current-page insertion and otherwise reports
the accumulated or split height maintained by the insertion subsystem; page
output clears the enquiry together with that subsystem state. The parity test
covers grouping, accumulation, splitting, absent classes, and output reset.

`\pdfximagebbox` reads typed, detached external-image metadata from the
checkpointed PDF ledger. PDF images retain the selected page-box coordinates
in scaled points; indices 1 through 4 return left, bottom, right, and top.
Raster images return `0.0pt` for every valid index. Missing image objects and
indices outside 1 through 4 are fatal with pdfTeX's pinned diagnostics. The
metadata registry is snapshot-, rollback-, and semantic-hash-safe and performs
no host I/O. `\pdfximage` scans page, page-box, and dimension options, opens a
host-neutral immutable raster or PDF-page source, allocates its typed object
identity, and updates `\pdflastximage`. `\pdfrefximage` appends a typed whatsit
that survives checkpoint, rollback, semantic hashing, artifact encoding, and
positioned shipout. PNG (gray/RGB/indexed/alpha), JPEG, and selected PDF pages
lower to typed image or form XObjects; repeated references reuse the same
object and PDF-page resources and transparency groups are recursively
remapped. Final PDF dictionaries, streams, resource entries, and content
operations are serialized exclusively through `pdf_writer` at the detached
output boundary.

The four PDF token parameters follow pdfTeX's distinct consumption scopes:
`\pdfpageattr` and `\pdfpageresources` are captured in each successful
shipout receipt, `\pdfpagesattr` is read when the final page-tree root is
assembled, and `\pdfpkmode` is fixed when PDF output is first initialized.
The 13 dimensions share the ordinary dimension scanner and display path.
`px` uses the live `\pdfpxdimen` only in pdfTeX mode; line height/depth
parameters apply during paragraph materialization with first/last overrides
of the each-line values and `\pdfignoreddimen` as the inactive sentinel. The
sentinel is fully live: pdfTeX also uses its current value for vertical-list
`prevdepth` initialization, comparison, and diagnostics. The committed
`pdf_ignored_dimen_effects` INITEX oracle proves both roles after changing the
sentinel away from its `-1000pt` default; TeX and original e-TeX retain their
fixed `-1000pt` constant.

The identity utility slice matches pdfTeX 1.40.27's token-level identity:
`\pdftexversion` is the internal integer 140, while `\pdftexrevision` and
`\pdftexbanner` expand to other-character tokens (and ordinary space tokens)
for patch level `.27` and the pinned TeX Live 2025 banner. Original primitive
meanings live in an immutable driver registry outside grouping and format
state. `\pdfprimitive` internalizes a frozen original-primitive token, and
`\ifpdfprimitive` requires both the original spelling and unchanged current
meaning; aliases and undefined names therefore test false. Format loading
reconstructs the registry without replacing shadowed live meanings.
`\ifpdfabsnum` and `\ifpdfabsdim` use the ordinary scanners and diagnostics,
then compare overflow-safe unsigned magnitudes for `<`, `=`, and `>`.
The four PDF string conversions and `\pdfstrcmp` scan expanded general text,
spell preserved control sequences with the live `\escapechar`, and operate on
pdfTeX bytes rather than Rust text ordering. Their results use space catcode
for byte 32 and other catcode for every other byte. Hex output is uppercase;
hex input ignores non-hex bytes and pads an unmatched final high nibble with
zero, matching the pinned pdfTeX 1.40.27 oracle.
`\pdfmatch` implements POSIX extended regular expressions over those same byte
strings, including `icase`, leftmost-longest matching, C-string NUL
termination, and the `subcount` capture limit. `\pdflastmatch` reports decimal
byte offsets and raw capture bytes. Capture state is checkpointed and hashed
but deliberately not grouped, matching pdfTeX's process-global match storage;
a no-match result clears capture availability, while a malformed expression
reports a recoverable warning and preserves the preceding successful state.
The timer, random seed, and shell-escape capability are immutable host inputs
at the `World` boundary plus checkpointed engine state. `\pdfelapsedtime` uses
pdfTeX's 16.16-second result at 100-microsecond resolution, and
`\pdfresettimer` rebases it without consulting a host clock during execution.
`\pdfsetrandomseed`, `\pdfuniformdeviate`, and `\pdfnormaldeviate` reproduce
pdfTeX's MetaPost-derived 55-word subtractive generator and fixed-point
rounding exactly. `\pdfshellescape` reports 0, 1, or 2 for disabled,
unrestricted, or restricted policy. Snapshots and semantic hashes include the
live timer and generator state; format images intentionally do not, so a
loaded format receives the new session's `World` inputs.
Creation time comes from the immutable job clock. File size, modification
date, byte dump, and file-mode MD5 enquiries resolve immutable content through
the same driver input policy as `\input`; expansion code never reads the host
filesystem or clock. `World` captures typed civil modification metadata with
each successful input record, and hermetic callers may seed it explicitly.
Missing content or missing modification metadata expands to nothing. File dump
offset and length default to zero, overlong ranges stop at EOF, and negative
ranges report recoverable pdfTeX diagnostics before being coerced to zero.
Raw PDF objects share the checkpointed page/font object ledger, so
`\pdfobj reserveobjnum`, `useobjnum`, ordinary, immediate, stream, attribute,
and file forms update `\pdflastobj` without a parallel counter. Valid object
operations leave `\pdfretval` unchanged. It starts at zero and
an invalid `\pdfobj useobjnum` sets the session-global value to sticky `-1`
before allocating the fallback object. The value participates in checkpoint
hashing and rollback, but format images intentionally omit it so a loaded
session starts again at zero, matching pdfTeX 1.40.27. Referenced and
immediate objects, document dictionary fragments, trailer fragments, and
custom trailer IDs are lowered only through the `pdf_writer` adapter; raw
syntax is confined to writer-framed object bodies or dictionary extension
entries. Repeated info, catalog, names, trailer, and trailer-ID token lists are
expanded when scanned and concatenated in source order. Final page-tree,
Names, catalog, and Info dictionary identities are allocated idempotently from
the same ledger in pdfTeX order. The
optional `\pdfcatalog ... openaction` suffix is scanned into a typed action
model shared with the later link and outline slices. Its action, destination,
structure, and forward-page identities are reserved immediately from that same
checkpointed ledger in pdfTeX order. Duplicate handling and DVI-mode
consumption match pdfTeX; finalization adds the typed catalog `/OpenAction`
reference and writer-framed action object without folding it into raw catalog
fragment bytes. The pinned `object_dictionaries` PDF fixture composes these
forms and document dictionaries, compares their normalized graph and exact
rendering with pdfTeX, and requires byte-identical retained-snapshot replay.
Its focused identity assertions require raw objects 1 and 2, action/forward
page identities 3 and 4, page resource/content identities 5 and 6, and final
document identities 7 through 10, including exact `useobjnum` and `\pdfrefobj`
preservation. Focused oracle tests require exact invalid-object warnings,
missing-reference and invalid-immediate diagnostics, DVI-mode errors, and
duplicate-open-action errors. Together with normalized and rendered PDF
fixtures plus retained-snapshot replay, these tests close the issue-13
object/dictionary parity gate.

The generated-font and microtype slice implements independent copied and
letterspaced font state, validated `\pdffontexpand` configuration, discrete
expansion-aware line breaking and final glyph/kern substitution, and signed
protrusion margin kerns. Generated and expanded identity survives snapshots,
formats, and detached page artifacts. Letterspaced virtual packets lower to
explicit movements around glyphs from the physical source font; PDF output
continues to use only the canonical `pdf_writer` serialization pipeline.
`\leftmarginkern` and `\rightmarginkern` implement pdfTeX's box-register edge
scan and exact void/non-hbox diagnostic.

The font-map state slice implements `\pdfmapfile`, `\pdfmapline`,
`\pdffontattr`, and `\pdfincludechars` as real pdfTeX-mode actions. Expanded
balanced text is converted to bytes and parsed without host I/O. Map-file and
font-program names remain logical resource names: native and WASM frontends
must acquire their bytes through the existing typed resource boundary.
Checkpointed append-only mutations give snapshot rollback and semantic hashes
the same exact suffix discipline as PDF object allocation. Map-line lookup
matches the pinned 1.40.27 observations: unprefixed and `+` duplicates preserve
the first entry, `=` replaces it, and `-` removes it. Duplicate warning
presentation is owned by child issue 17.1. Embedded dictionaries and custom
encoding vectors are serialized only through the canonical vendored
`pdf_writer` adapter. The 17.5 resource boundary accepts already acquired PFB,
TrueType SFNT, and PostScript encoding bytes under their logical map names,
validates them, and records stable SHA-256 program identities. PFB transport
framing is stripped while retaining all three PDF Type-1 segment lengths. Final PDF
assembly can resolve only this typed state; it never opens the host filesystem
or network. The native driver acquires exact mapline program names through
its configured `TEXFONTS` search path before finalization; other frontends can
provide the same typed resource by logical name. Font dictionaries,
program-derived descriptors, TFM-derived widths, custom `/Encoding`
differences, embedded `/FontFile` and `/FontFile2` streams, per-page `/Font`
resources, and absolute text operators are emitted through the detached graph
and canonical `pdf_writer` serializer. Font resource names and dictionary
object numbers are allocated at enquiry or first shipout use and survive
checkpoint rollback exactly. Finalization projects the union of committed page
glyph use and `\pdfincludechars` through the selected encoding. Type-1 eexec
programs are decrypted, reduced to the named CharStrings plus `.notdef`, and
deterministically re-encrypted; TrueType glyf programs use a compact named-glyph
and composite closure. Both reproduce pdfTeX's MD5-derived six-letter subset
name. Positive `\pdfgentounicode` emits UTF-16BE CMaps from global or
`tfm:name/glyph` mappings, while per-font `\pdfnobuiltintounicode` suppresses
the stream and nonzero `\pdfomitcharset` suppresses eligible Type-1 `/CharSet`.
All dictionaries and streams continue through the canonical vendored
`pdf_writer` graph.

## Compatibility and alias decisions

The implementation should preserve exact pdfTeX spellings even when a shared
engine-neutral implementation exists. In particular, `\pdfcreationdate`,
`\pdffilesize`, `\pdfshellescape`, and `\pdfstrcmp` should initially be aliases
over Umber's existing neutral facilities, with pdfTeX-compatible results and
error behavior. `\pdfcreationdate`/`\creationdate`,
`\pdffilesize`/`\filesize`, and `\pdfstrcmp`/`\strcmp` share their exact
implementations. `\pdfoptionpdfminorversion` is a legacy alias for
`\pdfminorversion` and shares its state cell. The two obsolete inclusion
controls instead retain the separate scan-time compatibility behavior
described above.

The source and executable audit found no blanket no-op among the potential
compatibility controls:

- `\quitvmode` is pdfTeX's third `start_par` command. It starts an indented
  paragraph in outer or internal vertical mode and consumes nothing in
  horizontal or math mode. `pdf_compatibility_controls` pins all three mode
  families.
- `\ignoreprimitiveerror` is a grouped integer bitmask. Bit 1 (the low bit) is
  consulted at the sole source use: infinite shrinkage encountered by
  `\vsplit`. Odd values print exactly `ignored error: Infinite glue shrinkage
  found in box being split` without entering ordinary error recovery, while
  the glue is still made finite. Even values keep the normal error and help.
- Positive `\pdfmovechars` warns when a not-yet-used PDF font is first marked
  used, then directly resets the live parameter to zero. Reassigning it before
  reusing only an already-used font neither warns nor resets it. The committed
  `pdf_move_chars_warning` INITEX oracle pins the warning, reset, and reuse
  distinction.
- `\pdfignoreddimen` is the live line-dimension and prevdepth sentinel
  described above; changing it is immediately observable.
- `\pdfpkmode` is fixed at the first committed PDF shipout and becomes part of
  the exact host-neutral `PdfPkFontRequest` together with the resolved DPI.
  Focused freezing tests and the 300/600-DPI Type3 fixtures use this typed
  request; it is not accepted-only state.
- `\pdfgamma`, `\pdfimagegamma`, `\pdfimagehicolor`, and
  `\pdfimageapplygamma` are fixed and clamped when PDF output opens. Pinned
  `writepng.c` uses apply-gamma plus the two gamma values for libpng sample
  conversion and uses high-color to retain or strip 16-bit samples (PDF below
  1.5 forces stripping). The native raster test now pins pdfTeX's exact output
  samples for a `gAMA=.5` grayscale ramp: disabled gamma preserves
  `00 01 11 22 33 44 55 66 77 88 99 aa bb cc dd ee ff`, while enabled gamma
  at `\pdfgamma=1000` and `\pdfimagegamma=2200` emits
  `00 00 01 05 0a 12 1c 29 38 49 5c 71 89 a3 c0 de ff`. It also proves that
  `\pdfimagehicolor=1` retains 16-bit samples only at PDF 1.5 or newer.
- The warning knobs have distinct predicates at their producers: duplicate
  destination and map warnings are suppressed only by positive values, while
  the page-group warning is suppressed by any nonzero value. Destination,
  map, and included-page-group integration are owned respectively by
  `umber2-kbz0.16.1`, closed `umber2-kbz0.17.1`, and
  `umber2-kbz0.14.1`.
- The omission controls all affect emitted dictionaries: nonzero
  `\pdfinfoomitdate` removes generated dates; odd `\pdfsuppressptexinfo`
  removes the pTeX banner; nonzero `\pdfomitcharset` removes eligible Type-1
  `/CharSet`; nonzero `\pdfomitinfodict` removes the Info object/reference;
  and `\pdfomitprocset` emits for negative values, follows the pre-PDF-2
  compatibility default at zero, and omits for positive values.
  `\pdfptexuseunderscore` selects `PTEX_` only when positive (or for PDF 2).
  Metadata and embedded-font PDF fixtures exercise these typed `pdf_writer`
  dictionary paths.

The audit above is tied to the pinned TeX Live source, rather than inferred
from parameter names. The exact producer map is:

| Control | Pinned pdfTeX 1.40.27 producer | Executable evidence |
| --- | --- | --- |
| `\quitvmode` | `pdftex.web:29876-29898` (`start_par`, subtype 2) | `pdf_compatibility_controls` covers vertical, horizontal, and math modes |
| `\ignoreprimitiveerror` | `pdftex.web:27668-27688` (low bit in `\vsplit`) | `pdf_compatibility_controls` covers ordinary and ignored recovery |
| `\pdfmovechars` | `pdftex.web:16089-16098` (`pdf_use_font`) | `pdf_move_chars_warning` covers first-use reset and already-used-font retention |
| `\pdfignoreddimen` | `pdftex.web:21415`, `23663`, and `25984-25998` | `pdf_ignored_dimen_effects` covers live `prevdepth` and line overrides |
| `\pdfpkmode` | `pdftex.web:19821-19825` | PK request/freezing tests plus the 300/600-DPI fixtures |
| PNG gamma/high color | `pdftex.web:15477-15480`; `writepng.c:524-545` | exact gamma ramp and PDF 1.4/1.5 16-bit tests |
| duplicate map warning | `mapfile.c:185-198` (`> 0` suppresses) | exact `-1/0/1` diagnostic test |
| page-group warning | `pdftoepdf.cc:934-936` (`!= 0` suppresses) | exact `-1/0/1` included-page-group test |
| duplicate destination warning | `pdftex.web:35015-35030` (`> 0` suppresses) | dependency `umber2-kbz0.16.1` owns both navigation collision timings |
| Info/date/pTeX controls | `pdftex.web:20341-20358`, `20427`, and `20463` | default, signed omission, odd suppression, key spelling, and Info removal tests |
| `\pdfomitcharset` | `writefont.c:487-515` | positive and negative Type-1 subset fixtures |
| `\pdfomitprocset` | `pdftex.web:19356-19359` | signed, zero, grouped, per-page, and PDF-2 tests |

## Beads epic decomposition

Create an epic titled **Implement the complete pdfTeX 1.40.27 primitive
layer**. Its acceptance criteria are: all 158 exact names are visible only in
the pdfTeX engine mode; every row above is done; focused reference fixtures
cover success, grouping, expansion, diagnostics, and mode errors; a
source-derived name-set test prevents omissions; DVI mode remains byte-stable;
PDF-mode fixtures match normalized pdfTeX structure and rendered pages; and
the full workspace, LaTeX, WASM, TRIP, and e-TRIP gates pass.

Create these child issues in dependency order; each issue should update this
checklist and include focused pdfTeX-oracle fixtures:

1. **Define pdfTeX engine mode and generated primitive inventory gate.** Add
   exact-name visibility, version identity, source-set comparison, and mode
   selection. This blocks every other issue.
2. **Design and implement the deterministic PDF artifact backend.** Add
   checkpointed object identities, page/resource dictionaries, deterministic
   serialization, normalized structural comparison, and rendering fixtures.
   This blocks backend-producing primitives.
3. **Add checkpointed pdfTeX parameter banks and format serialization.** Add
   typed integer, dimension, and token-list storage, grouping, semantic
   hashing, and format round trips. This depends on issue 1.
4. **Implement PDF page attributes, geometry, and origin parameters.** Cover
   the 4 token lists and 13 dimensions, with page-boundary behavior. Depends
   on issues 2 and 3.
5. **Implement PDF output, version, compression, and legacy aliases.** Cover
   output mode, major/minor version, compression, decimal digits, page box,
   and the three `\pdfoption...` aliases. Depends on issue 3.
6. **Implement PDF image, inclusion, and draft configuration parameters.**
   Cover resolution, gamma, inclusion policy, copy-fonts, draft mode, and
   unique resource names. The oracle-backed state/range child is complete;
   representative image effects remain dependency-ordered after issue 14.
   Depends on issue 3.
7. **Implement PDF microtype and font-output configuration parameters.**
   Cover adjustment, protrusion, kern insertion, tracing, ToUnicode, PK
   resolution, and character-set omission. The oracle-backed configuration
   contract and tracing diagnostic are complete in issue 7.1. Issue 7.2 has
   integrated paragraph shaping, Type-1 font-dictionary effects, and the
   host-neutral PK-to-Type3 bitmap pipeline after issues 10, 11, and 17.
   Depends on issue 3.
8. **Implement PDF metadata and warning-policy configuration parameters.**
   Cover date/info/procset omission, duplicate warnings, pTeX information,
   underscore policy, and compatibility-only parameter behavior. Depends on
   issue 3.
9. **Implement pdfTeX identity, utility conversions, and conditionals.** Cover
   version/banner, primitive recovery, absolute comparisons, escape/unescape,
   files, hashes, regex, timer, random, shell status, and neutral aliases.
10. **Implement pdfTeX character-code tables and font enquiries.** Cover the
   nine code tables, margin kerns, font name/object/size, inclusion height,
   image bounding boxes, and `\pdfnoligatures`.
11. **Implement letterspaced/copied fonts and expansion/protrusion.** Cover
   `\letterspacefont`, `\pdfcopyfont`, `\pdffontexpand`, protrusion parameters,
   and paragraph/line-breaking integration.
12. **Implement PDF graphics state, literals, color stacks, and positions.**
   Cover literal/matrix/save/restore, color stacks, save position, snapping,
   and last-position enquiries. Depends on issue 2.
13. **Implement PDF objects and document dictionaries.** Cover object/refobj,
   info/catalog/names/trailer/trailer ID, compression/version parameters, and
   last-object enquiry. Depends on issue 2.
14. **Implement PDF forms and external images.** Cover xform/ximage creation,
   references, names, image metadata, page-box/inclusion controls, and last
   enquiries. Depends on issues 2 and 13.
15. **Implement PDF annotations and link lifecycles.** Cover annotations,
    link start/end, margins, running-link controls, and last enquiries.
    Depends on issues 2 and 13.
16. **Implement PDF destinations, outlines, and article threads.** Cover
    destination and outline actions plus thread start/end and margins.
    Depends on issues 2 and 13.
17. **Implement PDF font maps, embedding, and ToUnicode controls.** Cover font
    attributes/maps, included characters, glyph mapping, built-in ToUnicode,
    and related omission controls. Depends on issues 10, 11, and 13.
18. **Implement PDF tagged-spacing and accessibility controls.** Cover
    fake/interword spaces, space fonts, character tag and boundary codes, and
    structure-sensitive output fixtures. Depends on issues 10, 11, and 17.
19. **Implement pdfTeX mode normalization and compatibility controls.** Cover
    `\quitvmode`, `\ignoreprimitiveerror`, and every compatibility/no-op
    candidate with oracle evidence. Depends on issues 5 through 9.
20. **Close the pdfTeX 1.40.27 full-parity gate.** Run the source-set audit,
    primitive micro-corpus, representative pdfTeX package/LaTeX corpus, PDF
    structural/rendering comparison, DVI regression, workspace tests, and
    WASM gates. Depends on all implementation issues.
