# LaTeX-DVI Support Contract

Status: implementation in progress  
Contract version: 1  
Reference distribution: TeX Live 2025 LaTeX2e kernel and base files

## Engine identity

`umber run --latex` selects the **Umber LaTeX-DVI** engine contract. It is an
explicit extension layer over Umber's e-TeX V2 mode and produces classic DVI.
It does not identify itself as pdfTeX, XeTeX, LuaTeX, or any other engine, and
it does not install another engine's identity primitive merely to satisfy a
feature probe.

The supported contract consists of:

- TeX82 semantics and primitives;
- the separately documented e-TeX V2 extension layer;
- the versioned Umber LaTeX extension inventory below;
- an Umber-native format built from pinned TeX Live 2025 LaTeX sources; and
- deterministic native and browser input resolution for that pinned closure.

Formats are Umber's validated semantic format images. TeX Live-native `.fmt`
files are not accepted. Loading a format never grants compatibility claims
beyond the driver mode selected for the run.

A fresh LaTeX-mode run starts from INITEX category codes rather than the
Plain-TeX-oriented defaults used by ordinary fresh runs. In particular, the
special syntax characters reset by `latex.ltx` begin with category `other`;
the kernel itself establishes its format-time category-code regime.

## Extension primitive inventory

These control sequences are visible only in LaTeX mode. They remain undefined
in TeX82 compatibility mode and plain e-TeX mode.

| Primitive | Status | Observable contract |
| --- | --- | --- |
| `\expanded` | done | Expands balanced text in the pdfTeX-manual message style: parameter characters are ordinary, protected macros remain unexpanded during the primitive's expansion, and the resulting tokens return to the surrounding expansion context. |
| `\filesize` | done | Expands to the decimal byte size of a resolved input file, or no tokens when the file is absent, through the same deterministic `World`-mediated lookup policy as `\input`. |

This inventory will grow only when the pinned LaTeX kernel or representative
base corpus demonstrates a semantic dependency. pdfTeX-prefixed aliases such
as `\pdffilesize` are intentionally omitted when a neutral primitive name is
accepted by the kernel.

## Compatibility and parity

Support means more than accepting LaTeX syntax. The pinned kernel must build a
byte-reproducible Umber-native format, and representative source-initialized
and format-loaded jobs must have identical effects and DVI. The base corpus
must match the pinned reference engine byte-for-byte in DVI after the existing
preamble-comment normalization and exactly in required multi-pass auxiliary
files.

The TeX82 TRIP, Plain TeX, and e-TeX/e-TRIP gates remain mandatory. LaTeX-only
meanings must not leak into either earlier mode.

During implementation, `scripts/discover-latex-kernel.sh` verifies the pinned
kernel hashes, runs the bootstrap with a fixed clock and search roots, and
reports the first recovered TeX diagnostic even when normal TeX recovery makes
the process exit successfully.

## Explicit non-goals

- PDF output or a native PDF backend;
- claiming pdfLaTeX primitive compatibility;
- unrestricted compatibility with the full CTAN package ecosystem;
- shell escape; and
- automatic execution of bibliography or index tools.

Auxiliary files used by external bibliography or index tools must still be
semantically exact where the supported corpus exercises them.
