# umber2-johp.165 — caller-independent missing-number recovery

Authority: TeX82 `tex.web` §§413, 416, 429, 440, 448, and 461.

`scan_something_internal` now owns §416 completely: it performs `back_error`,
reports the exact Missing number diagnostic and help, commits a `dimen_val`
zero, and lowers it through §429 before returning its ordinary value. The
integer, dimension, and glue callers no longer repeat that recovery.

`scan_dimen` has one result-observation exit for EOF, §416, and vacuous
non-internal recovery. The scalar microfixture covers token-list and font
operands through all three callers, asserting one diagnostic, replay, and
ordered internal/scalar observations; the focused scanner suite also covers
the §455 and ordinary continuation controls.
