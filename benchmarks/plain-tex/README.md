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
| `pages.tex` | vertical-list contribution, marks, insertions, page breaking, and the output routine |
| `dvi.tex` | traversal and DVI serialization of dense, nested, multi-font pages |

Each file has one workload size. A successful run writes its expected
`BENCHMARK` completion marker to the transcript and finishes with a DVI file.
The runner checks the marker and verifies that every measured run has the same
DVI checksum as its warm-up after removing the timestamp-bearing DVI preamble
comment.

Run the complete suite with the reference `tex` found on `PATH`:

```bash
scripts/bench-plain-tex.sh
```

Select another command-compatible TeX binary with `TEX_BIN`. The command must
accept `-interaction=batchmode`, load the Plain format, and produce DVI:

```bash
TEX_BIN=/path/to/tex scripts/bench-plain-tex.sh
```

The script performs one unmeasured warm-up and five measured runs per input.
It reports elapsed seconds from the shell's monotonic `time` keyword. Run it
on an otherwise idle machine and compare complete result sets from the same
host. Startup and filesystem costs are deliberately included.

The checked completion markers catch truncation and execution errors. They do
not replace byte-parity testing: use the repository's fixture, corpus, and
TRIP workflows for compatibility claims.
