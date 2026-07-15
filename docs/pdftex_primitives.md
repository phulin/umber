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
| Font construction and primitive recovery | 3 | partial (generated fonts done) | `\letterspacefont`, `\pdfcopyfont` (done); `\pdfprimitive` |
| Read-only integer enquiries | 14 | missing | `\pdftexversion`, `\pdflastobj`, `\pdflastxform`, `\pdflastximage`, `\pdflastximagepages`, `\pdflastannot`, `\pdflastxpos`, `\pdflastypos`, `\pdfretval`, `\pdflastximagecolordepth`, `\pdfelapsedtime`, `\pdfshellescape`, `\pdfrandomseed`, `\pdflastlink` |
| Expandable conversions and enquiries | 27 | partial (identity and font enquiries done) | `\expanded`, `\pdftexrevision`, `\pdftexbanner`, `\pdffontsize`, `\pdffontname`, `\pdffontobjnum`, `\leftmarginkern`, `\rightmarginkern` (done); `\pdfpageref`, `\pdfxformname`, `\pdfescapestring`, `\pdfescapename`, `\pdfescapehex`, `\pdfunescapehex`, `\pdfcreationdate`, `\pdffilemoddate`, `\pdffilesize`, `\pdfmdfivesum`, `\pdffiledump`, `\pdfmatch`, `\pdflastmatch`, `\pdfstrcmp`, `\pdfcolorstackinit`, `\pdfuniformdeviate`, `\pdfnormaldeviate`, `\pdfinsertht`, `\pdfximagebbox` |
| Primitive-identity conditional | 1 | missing | `\ifpdfprimitive` |
| Horizontal-mode normalization | 1 | missing | `\quitvmode` |
| Character codes and ligature control | 10 | done | `\lpcode`, `\rpcode`, `\efcode`, `\tagcode`, `\knbscode`, `\stbscode`, `\shbscode`, `\knbccode`, `\knaccode`, `\pdfnoligatures` |
| PDF backend actions | 43 | partial (font expansion and font-map state done) | `\pdffontexpand`, `\pdfincludechars`, `\pdffontattr`, `\pdfmapfile`, `\pdfmapline` (done); `\pdfliteral`, `\pdfcolorstack`, `\pdfsetmatrix`, `\pdfsave`, `\pdfrestore`, `\pdfobj`, `\pdfrefobj`, `\pdfxform`, `\pdfrefxform`, `\pdfximage`, `\pdfrefximage`, `\pdfannot`, `\pdfstartlink`, `\pdfendlink`, `\pdfoutline`, `\pdfdest`, `\pdfthread`, `\pdfstartthread`, `\pdfendthread`, `\pdfsavepos`, `\pdfsnaprefpoint`, `\pdfsnapy`, `\pdfsnapycomp`, `\pdfinfo`, `\pdfcatalog`, `\pdfnames`, `\pdftrailer`, `\pdftrailerid`, `\pdfresettimer`, `\pdfsetrandomseed`, `\pdfglyphtounicode`, `\pdfnobuiltintounicode`, `\pdfinterwordspaceon`, `\pdfinterwordspaceoff`, `\pdffakespace`, `\pdfrunninglinkoff`, `\pdfrunninglinkon`, `\pdfspacefont` |
| Compatibility error policy | 1 | missing | `\ignoreprimitiveerror` |
| Late expansion conditionals | 3 | partial (1 done) | `\ifincsname` (done); `\ifpdfabsnum`, `\ifpdfabsdim` |

Counts in the table sum to 158. No primitive is complete merely because its
name is installed: assignments must group correctly, expandable results and
diagnostics must match the pinned oracle, and node/effect state must survive
checkpoint, restore, semantic hashing, and format serialization where
applicable.

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
control. Those image effects, live page-box selection, and draft publication
behavior are tracked after external-image issue 14 by issue 6.2.

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

The actual expansion/protrusion, character-code-driven glue/kern, ToUnicode,
PK-font, and font-dictionary effects remain deliberately unclaimed: issue
7.2 depends on character tables (10), font shaping/expansion (11), and font
maps/embedding (17). Those backends must consume the typed live projection and
continue to serialize PDF through the canonical `pdf_writer` adapter. This
split prevents the configuration issue from duplicating the owning font
subsystems.

The four PDF token parameters follow pdfTeX's distinct consumption scopes:
`\pdfpageattr` and `\pdfpageresources` are captured in each successful
shipout receipt, `\pdfpagesattr` is read when the final page-tree root is
assembled, and `\pdfpkmode` is fixed when PDF output is first initialized.
The 13 dimensions share the ordinary dimension scanner and display path.
`px` uses the live `\pdfpxdimen` only in pdfTeX mode; line height/depth
parameters apply during paragraph materialization with first/last overrides
of the each-line values and `\pdfignoreddimen` as the inactive sentinel.

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
checkpoint rollback exactly. Subset program rewriting and ToUnicode streams
remain owned by child 17.3.

## Compatibility and alias decisions

The implementation should preserve exact pdfTeX spellings even when a shared
engine-neutral implementation exists. In particular, `\pdfcreationdate`,
`\pdffilesize`, `\pdfshellescape`, and `\pdfstrcmp` should initially be aliases
over Umber's existing neutral facilities, with pdfTeX-compatible results and
error behavior. `\pdfoptionpdfminorversion` is a legacy alias for
`\pdfminorversion` and shares its state cell. The two obsolete inclusion
controls instead retain the separate scan-time compatibility behavior
described above.

Potential compatibility/no-op controls are `\pdfmovechars`, the image gamma
controls, `\pdfignoreddimen`, `\pdfpkmode`, and warning-suppression or omission
knobs. They may be implemented as accepted state only where the pinned oracle
demonstrates that output is unaffected. Silently accepting a primitive whose
value or diagnostics remain observable is not parity.

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
   contract and tracing diagnostic are complete in issue 7.1; effective
   shaping and font-output integration remains in issue 7.2 after issues 10,
   11, and 17. Depends on issue 3.
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
