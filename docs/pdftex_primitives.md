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

The two fully implemented pdfTeX-layer names are `\expanded` and
`\ifincsname`. The 55 parameter names have typed, assignable state, but their
downstream PDF and typesetting effects remain assigned to checklist issues
4--8; the other 101 exact names still lack their final semantics.
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
| PDF token-list parameters | 4 | partial (state done) | `\pdfpagesattr`, `\pdfpageattr`, `\pdfpageresources`, `\pdfpkmode` |
| PDF integer parameters | 38 | partial (state done) | `\pdfoutput`, `\pdfcompresslevel`, `\pdfobjcompresslevel`, `\pdfdecimaldigits`, `\pdfmovechars`, `\pdfimageresolution`, `\pdfpkresolution`, `\pdfuniqueresname`, `\pdfoptionpdfminorversion`, `\pdfoptionalwaysusepdfpagebox`, `\pdfoptionpdfinclusionerrorlevel`, `\pdfmajorversion`, `\pdfminorversion`, `\pdfforcepagebox`, `\pdfpagebox`, `\pdfinclusionerrorlevel`, `\pdfgamma`, `\pdfimagegamma`, `\pdfimagehicolor`, `\pdfimageapplygamma`, `\pdfadjustspacing`, `\pdfprotrudechars`, `\pdftracingfonts`, `\pdfadjustinterwordglue`, `\pdfprependkern`, `\pdfappendkern`, `\pdfgentounicode`, `\pdfdraftmode`, `\pdfinclusioncopyfonts`, `\pdfsuppresswarningdupdest`, `\pdfsuppresswarningdupmap`, `\pdfsuppresswarningpagegroup`, `\pdfinfoomitdate`, `\pdfsuppressptexinfo`, `\pdfomitcharset`, `\pdfomitinfodict`, `\pdfomitprocset`, `\pdfptexuseunderscore` |
| PDF dimension parameters | 13 | partial (state done) | `\pdfhorigin`, `\pdfvorigin`, `\pdfpagewidth`, `\pdfpageheight`, `\pdflinkmargin`, `\pdfdestmargin`, `\pdfthreadmargin`, `\pdffirstlineheight`, `\pdflastlinedepth`, `\pdfeachlineheight`, `\pdfeachlinedepth`, `\pdfignoreddimen`, `\pdfpxdimen` |
| Font construction and primitive recovery | 3 | missing | `\letterspacefont`, `\pdfcopyfont`, `\pdfprimitive` |
| Read-only integer enquiries | 14 | missing | `\pdftexversion`, `\pdflastobj`, `\pdflastxform`, `\pdflastximage`, `\pdflastximagepages`, `\pdflastannot`, `\pdflastxpos`, `\pdflastypos`, `\pdfretval`, `\pdflastximagecolordepth`, `\pdfelapsedtime`, `\pdfshellescape`, `\pdfrandomseed`, `\pdflastlink` |
| Expandable conversions and enquiries | 27 | partial (1 done) | `\expanded` (done); `\pdftexrevision`, `\pdftexbanner`, `\pdffontname`, `\pdffontobjnum`, `\pdffontsize`, `\pdfpageref`, `\leftmarginkern`, `\rightmarginkern`, `\pdfxformname`, `\pdfescapestring`, `\pdfescapename`, `\pdfescapehex`, `\pdfunescapehex`, `\pdfcreationdate`, `\pdffilemoddate`, `\pdffilesize`, `\pdfmdfivesum`, `\pdffiledump`, `\pdfmatch`, `\pdflastmatch`, `\pdfstrcmp`, `\pdfcolorstackinit`, `\pdfuniformdeviate`, `\pdfnormaldeviate`, `\pdfinsertht`, `\pdfximagebbox` |
| Primitive-identity conditional | 1 | missing | `\ifpdfprimitive` |
| Horizontal-mode normalization | 1 | missing | `\quitvmode` |
| Character codes and ligature control | 10 | missing | `\lpcode`, `\rpcode`, `\efcode`, `\tagcode`, `\knbscode`, `\stbscode`, `\shbscode`, `\knbccode`, `\knaccode`, `\pdfnoligatures` |
| PDF backend actions | 43 | missing | `\pdfliteral`, `\pdfcolorstack`, `\pdfsetmatrix`, `\pdfsave`, `\pdfrestore`, `\pdfobj`, `\pdfrefobj`, `\pdfxform`, `\pdfrefxform`, `\pdfximage`, `\pdfrefximage`, `\pdfannot`, `\pdfstartlink`, `\pdfendlink`, `\pdfoutline`, `\pdfdest`, `\pdfthread`, `\pdfstartthread`, `\pdfendthread`, `\pdfsavepos`, `\pdfsnaprefpoint`, `\pdfsnapy`, `\pdfsnapycomp`, `\pdfinfo`, `\pdfcatalog`, `\pdfnames`, `\pdfincludechars`, `\pdffontattr`, `\pdfmapfile`, `\pdfmapline`, `\pdftrailer`, `\pdftrailerid`, `\pdfresettimer`, `\pdfsetrandomseed`, `\pdffontexpand`, `\pdfglyphtounicode`, `\pdfnobuiltintounicode`, `\pdfinterwordspaceon`, `\pdfinterwordspaceoff`, `\pdffakespace`, `\pdfrunninglinkoff`, `\pdfrunninglinkon`, `\pdfspacefont` |
| Compatibility error policy | 1 | missing | `\ignoreprimitiveerror` |
| Late expansion conditionals | 3 | partial (1 done) | `\ifincsname` (done); `\ifpdfabsnum`, `\ifpdfabsdim` |

Counts in the table sum to 158. No primitive is complete merely because its
name is installed: assignments must group correctly, expandable results and
diagnostics must match the pinned oracle, and node/effect state must survive
checkpoint, restore, semantic hashing, and format serialization where
applicable.

The parameter-state slice uses reserved cells in the existing typed integer,
dimension, and token-list banks. pdfTeX mode initializes the cells to the
pinned INITEX defaults; other engine modes leave them untouched. The three
legacy `\pdfoption...` spellings share cells with their current counterparts.
Assignments therefore inherit TeX grouping, `\global`, and `\globaldefs`
semantics from the common environment barrier, while snapshots, semantic
hashes, and format images include the values through the existing bank
machinery. Downstream consumers must not introduce shadow parameter state.

## Compatibility and alias decisions

The implementation should preserve exact pdfTeX spellings even when a shared
engine-neutral implementation exists. In particular, `\pdfcreationdate`,
`\pdffilesize`, `\pdfshellescape`, and `\pdfstrcmp` should initially be aliases
over Umber's existing neutral facilities, with pdfTeX-compatible results and
error behavior. `\pdfoptionpdfminorversion`,
`\pdfoptionpdfinclusionerrorlevel`, and `\pdfoptionalwaysusepdfpagebox` are
legacy aliases for their current parameter counterparts and should share one
state cell per pair.

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
   unique resource names. Depends on issue 3.
7. **Implement PDF microtype and font-output configuration parameters.**
   Cover adjustment, protrusion, kern insertion, tracing, ToUnicode, PK
   resolution, and character-set omission. Depends on issue 3.
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
