# LaTeX Engine Support Contract

Status: LaTeX-DVI supported; pdfLaTeX mode and deterministic format supported
Contract version: 2
Reference distribution: TeX Live 2025 LaTeX2e kernel and base files

## Engine identity

`umber run --latex` selects the **Umber LaTeX-DVI** engine contract. It is an
explicit extension layer over Umber's e-TeX V2 mode and produces classic DVI.
It does not identify itself as pdfTeX, XeTeX, LuaTeX, or any other engine, and
it does not install another engine's identity primitive merely to satisfy a
feature probe.

`umber run --pdflatex` selects the **Umber pdfLaTeX** engine contract. It
composes that same LaTeX compatibility layer with the complete pdfTeX layer;
the composition owns LaTeX's byte-oriented UTF-8 input behavior and can
publish PDF through the deterministic PDF backend. `--latex` and `--pdflatex`
remain distinct because the selected engine contract controls primitive
visibility even after a format image is loaded.

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

These neutral control sequences are visible in both LaTeX contracts. They
remain undefined in TeX82 compatibility mode and plain e-TeX mode; pdfLaTeX
additionally exposes the documented pdfTeX-prefixed primitive surface.

| Primitive       | Status | Observable contract                                                                                                                                                                                                                     |
| --------------- | ------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `\expanded`     | done   | Expands balanced text in the pdfTeX-manual message style: parameter characters are ordinary, protected macros remain unexpanded during the primitive's expansion, and the resulting tokens return to the surrounding expansion context. |
| `\filesize`     | done   | Expands to the decimal byte size of a resolved input file, or no tokens when the file is absent, through the same deterministic `World`-mediated lookup policy as `\input`.                                                             |
| `\strcmp`       | done   | Fully expands two balanced texts, converts their tokens with TeX string-printing rules, and returns `-1`, `0`, or `1` from bytewise lexical comparison.                                                                                 |
| `\shellescape`  | done   | Expands to `0`, reporting the supported driver's always-disabled shell-escape policy without executing or authorizing host commands.                                                                                                    |
| `\creationdate` | done   | Expands to the immutable UTC job-start timestamp as `D:YYYYMMDDhhmmssZ`; native runs derive it from the pinned `SOURCE_DATE_EPOCH`, and format-loaded jobs do not consult mutable TeX clock parameters.                                 |

This inventory will grow only when the pinned LaTeX kernel or representative
base corpus demonstrates a semantic dependency. pdfTeX-prefixed aliases such
as `\pdffilesize` are intentionally omitted when a neutral primitive name is
accepted by the kernel.

## Compatibility and parity

Support means more than accepting LaTeX syntax. Each pinned kernel mode must
build a byte-reproducible Umber-native format. Representative
source-initialized and format-loaded jobs must have identical effects and
driver output: DVI for `latex.fmt`, PDF for `pdflatex.fmt`. PDF formats retain
the kernel's canonical glyph-to-Unicode mappings while rejecting live pages,
objects, resources, or other document-local PDF state at `\dump`. The DVI base corpus
must match the pinned reference engine byte-for-byte in DVI after the existing
preamble-comment normalization and exactly in required multi-pass auxiliary
files.

The TeX82 TRIP, Plain TeX, and e-TeX/e-TRIP gates remain mandatory. LaTeX-only
meanings must not leak into either earlier mode.

During implementation, `scripts/discover-latex-kernel.sh` verifies the pinned
kernel and Unicode-data hashes, runs the bootstrap with a fixed clock and
explicit source/font search roots, and reports the first recovered TeX
diagnostic even when normal TeX recovery makes the process exit successfully.

## Supported workflows

### CLI

Build the pinned format and run a format-loaded LaTeX-DVI job with explicit
TeX Live lookup roots:

```sh
scripts/build-latex-format.sh
TEXINPUTS=/usr/local/texlive/2025/texmf-dist/tex/latex/base:/usr/local/texlive/2025/texmf-dist/tex/latex/l3kernel:/usr/local/texlive/2025/texmf-dist/tex/latex/l3backend \
TEXFONTS=/usr/local/texlive/2025/texmf-dist/fonts/tfm/public/cm \
  cargo run-dev -p umber -- run --latex document.tex \
    --format target/latex-format/latex.fmt --dvi document.dvi
```

`--latex` is the engine contract selector; `--format` must name an
Umber-generated image. The output is DVI, never PDF. Repeat the command when a
document needs multiple AUX/TOC passes.

Build and run the corresponding pdfLaTeX mode with the same pinned common
source closure plus its explicitly locked PDF configuration inputs:

```sh
scripts/build-latex-format.sh --engine pdflatex
TEXINPUTS=/usr/local/texlive/2025/texmf-dist/tex/latex/base:/usr/local/texlive/2025/texmf-dist/tex/latex/l3kernel:/usr/local/texlive/2025/texmf-dist/tex/latex/l3backend \
TEXFONTS=/usr/local/texlive/2025/texmf-dist/fonts/tfm/public/cm \
  cargo run-dev -p umber -- run --pdflatex document.tex \
    --format target/pdflatex-format/pdflatex.fmt --pdf document.pdf
```

The builder performs two clean format generations and exact source-versus-
format PDF and auxiliary-file equivalence. Cross-engine pdfTeX parity uses
normalized structure and rendered pages rather than requiring serializer-byte
identity.

### Rust library

Source initialization selects the extension contract explicitly with
`umber::prepare_latex_run_stores(&mut stores)` or
`umber::prepare_pdflatex_run_stores(&mut stores)`. Normal applications should load
the pinned bytes with `Universe::from_format(world, &format_bytes)` and execute
through `EngineSession` plus `FileSessionResolvers`; this is the same composed
path used by the CLI. A library embedding remains responsible for labeling the
run LaTeX-DVI and for supplying deterministic TeX/TFM search areas.

Library callers that need automatic bibliography processing opt into
`umber::LatexProjectSession`. It runs the existing TeX engine and in-process
`BibSession`, targeting Biber 2.22 beta, control schema 3.11, and BBL schema
3.3. Ordinary and tool processing, control validation, alternate bibliography
outputs, and TeX-bibliography-TeX convergence use the same pure-Rust pipeline
in native and WASM builds. The project session returns typed combined resource
needs and atomically accepts the root, generated files, bibliography
diagnostics, auxiliary files, and final DVI/HTML. The single-pass APIs retain
their existing meaning.

### npm and browser worker

Publish the format and exact base-corpus input closure, then select the named
format in the standard worker resolver:

```sh
scripts/build-wasm-latex-bundle.sh \
  --objects-base-url https://cdn.example/umber/latex/objects/
scripts/check-latex-wasm.sh
```

```js
const files = new Map([
  ["document.tex", new TextEncoder().encode(
    "\\documentclass{article}\\begin{document}Hello.\\end{document}",
  )],
  ["document.aux", new Uint8Array()],
]);
const output = await compileInWorker(
  { mainPath: "document.tex", jobName: "document" },
  files,
  { manifestUrl, format: "latex", persistentCache: "indexeddb" },
);
```

The empty AUX file represents a clean first pass; feed emitted AUX/TOC files
back as user files for later passes. The manifest resolver verifies the format
and every TeX/TFM object by length and SHA-256 before the WASM engine sees it.
The package example under `examples/latex.html` is the same browser workflow.

### Verification tiers

- `scripts/check-latex-corpus.sh` runs article, report, book, and letter for
  three passes against TeX Live 2025 and verifies the exact 30-file runtime
  closure.
- `scripts/check-latex-wasm.sh` builds the package and hosted bundle, runs the
  article corpus for three passes inside the generated WASM module, and
  requires exact native DVI/AUX/TOC parity.
- `scripts/check-latex-parity.sh` derives a whole-repository LaTeX2e DVI cohort
  from one hash-pinned upstream archive and runs it against TeX Live 2025. The
  pinned scopes yield 295 shipout-log candidates: 286 actual classic-LaTeX
  DVIs and nine alternate test configurations. One reference-DVI case,
  `base/testfiles/sx172785.lvt`, is retained in the census but explicitly
  skipped because it requires the unsupported pdfTeX-only
  `\pdfprotrudechars` and `\rpcode` primitives. All 285 applicable cases are
  coordinate-exact. The manifest pins the exact nine non-DVI paths; an
  unexpected missing or present reference DVI fails instead of changing a
  case's classification. The checker builds one verified format before the suite,
  or accepts one with `--format`, then restores byte-identical copies of that
  pregenerated image in 285 fresh case sessions. Logs are diagnostic only;
  after preamble-comment normalization, every DVI byte and therefore every
  encoded coordinate must match. Reference kpathsea lookup is run with a clean
  environment and explicit TeX, TFM, configuration, and format paths: inputs
  may come only from the pinned upstream snapshot, the selected TeX Live 2025
  `texmf-dist`, the exact distribution-owned `latex.fmt` and configuration, or
  that case's isolated generated-state directory. Every recorder input is
  canonicalized after the run, including through symlinks, and an input outside
  those roots fails the case before any discovered directory is mirrored into
  Umber. `--self-test-reference-lookup` checks that enforcement without needing
  TeX Live. The checker writes separate persistent
  failure, skip, non-DVI, and format-receipt reports. Unless `--keep-work` is
  requested, it removes each completed case directory immediately and removes
  the run root on success, failure, or interruption. Run
  `scripts/setup-latex-parity-tests.sh` first, or use both commands with
  `--offline` after the archive is cached.
- `scripts/discover-latex-kernel.sh` remains the progressive bootstrap
  diagnostic when the pinned kernel changes.

## Explicit non-goals

- whole-upstream-corpus pdfLaTeX structural and rendering parity in this
  initial engine-mode milestone;
- unrestricted compatibility with the full CTAN package ecosystem;
- shell escape; and
- automatic execution of index tools.

Auxiliary files used by external index tools must still be semantically exact
where the supported corpus exercises them. Bibliography automation is exposed
only through the explicit project-session API, not implicit single-pass CLI
execution.
