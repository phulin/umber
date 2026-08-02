# umber2-johp.462 — line-break short-display font lifetime

Authority: TeX82 `tex.web` §§174 and 851.

Section 174's `font_in_short_display` is session state: a character emits its
font identifier only when that font differs from the one retained by the
preceding abbreviated fragment. Section 851 initializes the state once while
creating the initial active breakpoint for each line-breaking pass. It does
not reset the state at every feasible breakpoint.

The `tex-exec` short-display renderer now owns that state explicitly.
Paragraph tracing retains one renderer across feasible-break fragments and
resets it on each pass event. Standalone packed-box diagnostics create a fresh
renderer, preserving their independent TeX82 §663 reset. A focused regression
covers both persistence and reset, and the change does not enter layout or
line-breaking decisions.

Guarded format-loaded TRIP advanced the gating log mismatch from byte 8185 to
byte 12033. Normalized DVI remains exact at 2920 bytes with SHA-256
`6420f3461dec8e5feed4b03bfc3717d00c8a36fae4fe9226f6d53a4db7592bb9`, and
all 22 command events remain exact with SHA-256
`1d4a6705f09d4c80c2bdc7aa3d1273cd09af1bc9341eba3c70c9c1ae04b863c1`.
The newly exposed `\box255` error-help transcript front is tracked by
`umber2-johp.466`.
