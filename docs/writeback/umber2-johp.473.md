# umber2-johp.473 — shipout box-constructor tracing

Authority: TeX82 `tex.web` §§1030, 1075, and 1084.

The `leader_ship` main-control case traces `\shipout` at §1030, then calls
`scan_box` internally. A valid `\hbox`, `\vbox`, or `\vtop` operand is
therefore scanner-owned and never returns to the `reswitch` trace boundary.

Split replay represents `\shipout` and its box constructor as adjacent
processor episodes. The existing pending-shipout state now also preserves the
canonical trace ownership boundary: it suppresses only the constructor's
standalone main-control trace, leaving the mode prefix for the first command
inside the box. A focused regression proves that the shipout constructor is
hidden while a following standalone constructor remains traced.

Guarded format-loaded TRIP advances the gating log mismatch from byte 13715
to byte 13838. The actual log SHA-256 changes from
`cc8ca665fd7539788bb02438e447e84f2fec39f1c2e4189851599125f9271417` to
`ec006e5986b1acd13838c5221e4b5dfc745bc38a9a01affc4bfd1802e8bfab2d`, while
normalized DVI and all 22 command events remain exact. The new diagnostic box
dump newline front is tracked by `umber2-johp.475`.
