# pdfTeX 1.40.27 primitives

This document is the complete source-derived inventory of the 158 primitive
control sequences added by pdfTeX on top of TeX82 and original e-TeX. Beads is
the source of truth for implementation progress; this file fixes the upstream
boundary and the delivery priority.

## Priority rule

**High priority** means the primitive executed in at least one of the 100
literal-random, source-available arXiv papers recorded in
`scripts/pdftex-arxiv-sample-100.tsv`. The trace ran each source with pdfTeX
1.40.27 and retained the primitives reached before errors in the 24 incomplete
runs. This identifies practical demand; it is not a claim that an unobserved
primitive is unused everywhere.

**Deferred** means the primitive was not in the 54 `\pdf...` names selected by
that trace. Deferred primitives remain part of the compatibility target, but
they do not block the high-priority digital-document milestone.

## Upstream boundary

The authoritative source is `texk/web2c/pdftexdir/pdftex.web` at TeX Live
commit `1664cf0ab3f6ce3b80db649bc6723f54ab12016c`. It declares pdfTeX
3.141592653-2.6-1.40.27 and has SHA-256
`5a105669acc1b49aedb7560d4d15cb2e23467cb16d895eb0031c8dd9fea32f04`.
The inventory is the unique names registered by `primitive(...)` in that file
minus the names registered by the matching TeX82 `tex.web` and original e-TeX
`etex.ch`.

| Layer | Primitive names |
| --- | ---: |
| TeX82 prerequisite | 325 |
| Original e-TeX prerequisite | 66 |
| pdfTeX layer inventoried here | 158 |
| Total in pdfTeX mode | 549 |

## High priority (54)

The observed-paper count is out of 100. An incomplete paper still contributes
the primitives it executed before stopping.

| Primitive | Papers | Primitive | Papers |
| --- | ---: | --- | ---: |
| `\pdfshellescape` | 100 | `\pdfstrcmp` | 100 |
| `\pdfoutput` | 94 | `\pdfmajorversion` | 92 |
| `\pdflastobj` | 91 | `\pdflastannot` | 90 |
| `\pdflastlink` | 90 | `\pdfpageheight` | 87 |
| `\pdfpagewidth` | 87 | `\pdftexversion` | 86 |
| `\pdfhorigin` | 83 | `\pdfvorigin` | 83 |
| `\pdfcolorstack` | 63 | `\pdflastximage` | 63 |
| `\pdfrefximage` | 63 | `\pdfximage` | 63 |
| `\pdfrestore` | 61 | `\pdfsave` | 61 |
| `\pdfsetmatrix` | 60 | `\pdftexrevision` | 57 |
| `\pdfoptionpdfminorversion` | 51 | `\pdfdraftmode` | 50 |
| `\pdflinkmargin` | 50 | `\pdfinfo` | 49 |
| `\pdfcatalog` | 47 | `\pdfdest` | 47 |
| `\pdfendlink` | 39 | `\pdfstartlink` | 39 |
| `\pdfobj` | 32 | `\pdfpageresources` | 31 |
| `\pdfliteral` | 20 | `\pdfmatch` | 18 |
| `\pdfadjustspacing` | 16 | `\pdffontexpand` | 16 |
| `\pdfprotrudechars` | 16 | `\pdflastxform` | 10 |
| `\pdfrefxform` | 10 | `\pdfxform` | 10 |
| `\pdfgentounicode` | 6 | `\pdfglyphtounicode` | 6 |
| `\pdfcolorstackinit` | 4 | `\pdfcompresslevel` | 3 |
| `\pdfdecimaldigits` | 3 | `\pdffontattr` | 3 |
| `\pdfoutline` | 3 | `\pdflastxpos` | 2 |
| `\pdflastypos` | 2 | `\pdfnobuiltintounicode` | 2 |
| `\pdfsavepos` | 2 | `\pdfadjustinterwordglue` | 1 |
| `\pdfappendkern` | 1 | `\pdffontsize` | 1 |
| `\pdfmapline` | 1 | `\pdfprependkern` | 1 |

## Deferred (104)

The lists retain the registration-family ordering from the pinned source.

| Family | Count | Primitive names |
| --- | ---: | --- |
| PDF token-list parameters | 3 | `\pdfpagesattr`, `\pdfpageattr`, `\pdfpkmode` |
| PDF integer parameters | 26 | `\pdfobjcompresslevel`, `\pdfmovechars`, `\pdfimageresolution`, `\pdfpkresolution`, `\pdfuniqueresname`, `\pdfoptionalwaysusepdfpagebox`, `\pdfoptionpdfinclusionerrorlevel`, `\pdfminorversion`, `\pdfforcepagebox`, `\pdfpagebox`, `\pdfinclusionerrorlevel`, `\pdfgamma`, `\pdfimagegamma`, `\pdfimagehicolor`, `\pdfimageapplygamma`, `\pdftracingfonts`, `\pdfinclusioncopyfonts`, `\pdfsuppresswarningdupdest`, `\pdfsuppresswarningdupmap`, `\pdfsuppresswarningpagegroup`, `\pdfinfoomitdate`, `\pdfsuppressptexinfo`, `\pdfomitcharset`, `\pdfomitinfodict`, `\pdfomitprocset`, `\pdfptexuseunderscore` |
| PDF dimension parameters | 8 | `\pdfdestmargin`, `\pdfthreadmargin`, `\pdffirstlineheight`, `\pdflastlinedepth`, `\pdfeachlineheight`, `\pdfeachlinedepth`, `\pdfignoreddimen`, `\pdfpxdimen` |
| Font construction and primitive recovery | 3 | `\letterspacefont`, `\pdfcopyfont`, `\pdfprimitive` |
| Read-only integer enquiries | 5 | `\pdflastximagepages`, `\pdfretval`, `\pdflastximagecolordepth`, `\pdfelapsedtime`, `\pdfrandomseed` |
| Expandable conversions and enquiries | 22 | `\expanded`, `\pdftexbanner`, `\pdffontname`, `\pdffontobjnum`, `\pdfpageref`, `\leftmarginkern`, `\rightmarginkern`, `\pdfxformname`, `\pdfescapestring`, `\pdfescapename`, `\pdfescapehex`, `\pdfunescapehex`, `\pdfcreationdate`, `\pdffilemoddate`, `\pdffilesize`, `\pdfmdfivesum`, `\pdffiledump`, `\pdflastmatch`, `\pdfuniformdeviate`, `\pdfnormaldeviate`, `\pdfinsertht`, `\pdfximagebbox` |
| Primitive-identity conditional | 1 | `\ifpdfprimitive` |
| Horizontal-mode normalization | 1 | `\quitvmode` |
| Character codes and ligature control | 10 | `\lpcode`, `\rpcode`, `\efcode`, `\tagcode`, `\knbscode`, `\stbscode`, `\shbscode`, `\knbccode`, `\knaccode`, `\pdfnoligatures` |
| PDF backend actions | 21 | `\pdfrefobj`, `\pdfannot`, `\pdfthread`, `\pdfstartthread`, `\pdfendthread`, `\pdfsnaprefpoint`, `\pdfsnapy`, `\pdfsnapycomp`, `\pdfnames`, `\pdfincludechars`, `\pdfmapfile`, `\pdftrailer`, `\pdftrailerid`, `\pdfresettimer`, `\pdfsetrandomseed`, `\pdfinterwordspaceon`, `\pdfinterwordspaceoff`, `\pdffakespace`, `\pdfrunninglinkoff`, `\pdfrunninglinkon`, `\pdfspacefont` |
| Compatibility error policy | 1 | `\ignoreprimitiveerror` |
| Late expansion conditionals | 3 | `\ifincsname`, `\ifpdfabsnum`, `\ifpdfabsdim` |

The family counts sum to 158 across the two priority sections. Exact-name
registration alone is not completion: assignments must group correctly,
expandable results and diagnostics must match the pinned oracle, PDF effects
must serialize deterministically, and relevant state must survive checkpoint,
restore, hashing, and format serialization.

## Delivery gates

The high-priority epic is complete when all 54 observed primitives have
oracle-backed behavior, representative digital-document sources complete,
normalized PDF structure and rendered pages match pdfTeX, DVI remains stable,
and the native/WASM gates pass.

The deferred epic owns the other 104 primitives and the final 158-name source
audit. It should not block the high-priority milestone unless a supposedly
deferred primitive is discovered to be a real dependency of the practical
corpus; such a primitive moves to high priority with the trace or fixture that
demonstrates the dependency.
