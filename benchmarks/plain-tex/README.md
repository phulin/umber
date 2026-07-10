# Plain TeX engine benchmarks

This directory contains fixed, deterministic end-to-end workloads for a
TeX82-compatible engine. They are performance benchmarks, not correctness
tests, and are intentionally excluded from `cargo test`.

| Input | Dominant work |
| --- | --- |
| `expand.tex` | macro argument matching, conditionals, `\csname`, token registers, and integer assignments |
| `paragraph-wide.tex` | natural-language paragraph construction and line breaking at a comfortable width |
| `paragraph-narrow.tex` | hyphenation and a larger active-breakpoint search at a narrow width |
| `math.tex` | math-list construction, style changes, fractions, radicals, delimiters, and packing |
| `math-nested.tex` | deeply nested fractions, radicals, scripts, and bottom-up box lowering |
| `pages.tex` | vertical-list contribution, marks, insertions, page breaking, and the output routine |
| `dvi.tex` | traversal and DVI serialization of dense, nested, multi-font pages |

Each file has one workload size. A successful run writes its expected
`BENCH` completion marker to the transcript and finishes with a DVI file.
External-engine artifacts are compared across runs after removing the
timestamp-bearing DVI preamble comment.

Run the complete comparison suite with no arguments:

```bash
scripts/bench-plain-tex.sh
```

The script builds release-mode Umber before timing, then automatically detects
and benchmarks installed `tex`, `pdftex`, `luatex`, and `xetex` commands.
pdfTeX and LuaTeX are forced into DVI mode; XeTeX is run in no-PDF mode and its
XDV is validated like the DVI artifacts. The final table reports the mean,
minimum, maximum, and ratio to stock `tex` (or the first available external
engine when `tex` is absent).

The script performs one unmeasured warm-up and five measured runs per
engine/input pair. It times only the engine invocation: release compilation,
input copying, completion checks, and artifact checksums are outside the timed
region. Startup and engine filesystem activity remain included. Run it on an
otherwise idle machine and compare complete result sets from the same host.

The workloads use a shared primitive-compatible preamble, committed Computer
Modern metrics, explicit register assignments, and explicit discretionary
breakpoints so every engine receives the same input. For external engines,
completion markers and normalized artifact checks catch truncation, execution
errors, and nondeterminism. Umber is deliberately best-effort during bring-up:
its invocation is timed even when it exits with an error, and its output is not
validated. The table marks how many of its five measured runs failed. These
checks do not replace cross-engine byte-parity testing: use the repository's
fixture, corpus, and TRIP workflows for compatibility claims.
