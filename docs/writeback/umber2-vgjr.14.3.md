# umber2-vgjr.14.3 — simplified classic BibTeX runtime

The explicit-frame VM remains the execution authority. BST scanning now writes
tokens, diagnostics, nesting, and work directly into compiler-owned state; the
separate lexer module and transfer result are deleted. One `Callable` value
represents quoted functions, builtins, and variables through compilation,
operand-stack control flow, and VM dispatch, so quoted values no longer create
synthetic compiled functions.

Classic `READ` expands source fields directly into compact entry-field-indexed
slots and performs crossref inheritance over those slots. The string-keyed raw
field projection is deleted. The prepared cache retains typed source, control,
style, and option inputs directly instead of allocating a second debug-string
identity. Its byte charge includes every input it keeps alive. VM diagnostics
live only in the ordered log-event stream; diagnostic consumers use a borrowed
projection, preserving report and BLG order without cloned records.

Measured against `91e227ee8`, production Rust adds 367 lines and deletes 497,
a net reduction of 130 lines. Focused proof adds 39 lines and deletes 17, so
the complete implementation is 406 additions and 514 deletions, a net
reduction of 108 lines. No fixtures or generated sources changed, and no
linked discovery was required. This 31-line writeback makes the total tracked
change 437 additions and 514 deletions, a net reduction of 77 lines.

The exact implementation passed uncapped focused and full-workspace `--no-run`
builds, focused `bib-engine` execution under `MemoryMax=512M` (72 unit and 356
compatibility tests passed; 940 declared ignores), and the complete routine
workspace under `MemoryMax=1G`. These gates cover the upstream classic
fixtures, Web2C allocation and pool traces, function semantics, exact BLG/BBL
bytes, diagnostics, cache replay, and execution limits. `scripts/check.sh`
passed all four gates under the 1 GiB cgroup.
