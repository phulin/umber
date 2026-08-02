# umber2-johp.475 — output-active vbox diagnostic newline

Authority: TeX82 `tex.web` §§182 and 675.

Section 675 deliberately places the vbox headline's `print_ln` inside its
non-output-active branch. While `\output` is active, the headline stays open
until §182's first `show_node_list` newline begins the packed-box dump. The two
paths therefore have different newline ownership even though both place the
first box node on the next line.

The pack reporter now closes a vbox headline itself only outside an active
output routine. Its diagnostic scope continues to supply §182's newline, so
output-active reports have one separator and ordinary reports retain two. A
focused two-case regression pins both branches.

Guarded format-loaded TRIP advances the gating log mismatch from byte 13838
to byte 23604. The actual log SHA-256 changes from
`ec006e5986b1acd13838c5221e4b5dfc745bc38a9a01affc4bfd1802e8bfab2d` to
`5fac77acb25dd6fb6ea8f6075276a86859a949290a01721a58d5043fb9b8bd69`, while
normalized DVI and all 22 command events remain exact. The new dimension-error
context front is tracked by `umber2-johp.482`.
