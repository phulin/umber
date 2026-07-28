# umber2-johp.250 — optional equals category check

TeX82 §405 compares the complete `cur_tok` with `other_token + '='`. The
canonical scalar scanner consequently accepts only a `Catcode::Other` equals
after spacer tokens. A same-character token at every other directly
deliverable category is backed up, and an active equals follows its active
control-sequence meaning before the same rejection and replay path.

`scanner_syntax_optional_equals_catcode_and_relax_boundaries` covers the
accepted Other case, spaces, all directly deliverable non-Other categories,
active-character expansion, following-token replay, and the one canonical
backup observation. The TeX82 catalogue records this evidence under
`tex82.scanner.syntax`; its remaining §403 relaxed-brace gap stays owned by
`umber2-johp.209`.
