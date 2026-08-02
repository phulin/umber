# umber2-johp.466 — integrated box255 context confirmation

Authority: TeX82 `tex.web` §§82 and 1015.

This issue required no engine change. The assigned base already contains
`umber2-johp.463`'s generic correction: §1015's nonvoid `\box255` diagnostic
retains the live triggering-command context required by §82 before printing
the error help. A fresh guarded format-loaded TRIP run confirms that the
recorded byte-12033 mismatch is absent.

The gating log front advanced to byte 12352. Normalized DVI remains exact at
2920 bytes with SHA-256
`6420f3461dec8e5feed4b03bfc3717d00c8a36fae4fe9226f6d53a4db7592bb9`, and
all 22 command events remain exact with SHA-256
`1d4a6705f09d4c80c2bdc7aa3d1273cd09af1bc9341eba3c70c9c1ae04b863c1`.
The newly exposed internal-vertical-mode diagnostic front is tracked by
`umber2-johp.468`.
