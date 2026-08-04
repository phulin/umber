# umber2-johp.38 — nested output-box group closure

Authority: TeX82 `tex.web` §§1016, 1025, and 1026 (`fire_up`, output-routine
entry, and page-builder resumption), confirmed unchanged by pdfTeX 1.40.29
`pdftex.web` §§1016, 1025, and 1026.

`fire_up` enters `output_group`, starts the output token list, and consumes its
required outer left brace before ordinary replay. A right brace belonging to an
active nested box body therefore closes that box before the output list's outer
right brace can select the `output_group` teardown. Canonical main control now
gives active box-body closure precedence over output teardown; this preserves
Plain's `\line{\vbox to8.5\p@{}}` and leaves §1026 to close only the enclosing
output group.
