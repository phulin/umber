# umber2-johp.274 — mandatory left-brace recovery

TeX82 §403 owns a missing mandatory left brace as one complete scanner
transition: it prints `Missing { inserted`, attaches the four canonical help
lines, backs up the offending command through `back_error`, installs the
synthetic left-brace meaning, and increments `align_state`.

`CommandProcessor::scan_left_brace` now returns a typed consumed-or-inserted
outcome instead of representing successful recovery as `InputInvariant`.
Every token-list, hyphenation, math-field, math-choice, box, insert, alignment,
and unexpanded-text caller therefore continues after the same shared recovery.
The rejected command keeps its spelling, source provenance, and ordinary
backup replay, while the inserted brace has unknown source provenance and is
accounted for only by the scanner-owned `align_state` increment.

The unit matrix covers exact diagnostic and help text, spaces, `\relax`,
expanded calls, valid-brace negative control, offender replay, brace
accounting, token-list collection, hyphenation data, and math-group opening.
The committed `missing-left-brace-recovery` scanner microfixture exercises
bare INITEX's other-category `{` through the box-opening caller.
