# umber2-johp.47 — canonical rule contribution and Story DVI

Authority: TeX82 `tex.web` §1095 (`head_for_vmode` and `append_to_vlist` for
`\hrule`, plus the horizontal-mode `end_graf` path), and §32 (`ship_out`
serializes `count(0)` through `count(9)` after `bop`). pdfTeX 1.40.27
`pdftex.web` retains the DVI route by dispatching `ship_out` to
`dvi_ship_out` when PDF output is disabled.

Canonical replay must not leave a vertical `\hrule` on the mode-nest list.
It contributes to the outer page, resets `prev_depth`, and invokes the page
builder; an ordinary horizontal `\hrule` first ends its paragraph, while a
restricted-horizontal one reports TeX's leaders-only diagnostic. `\vrule`
instead starts horizontal mode when necessary and appends there. This restores
the rule-bearing first Story page whose `bop` count operands begin with
`count(0)=1`.
