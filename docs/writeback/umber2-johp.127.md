# umber2-johp.127 — read-to dropped effective definition scope

TeX82 §1214 resolves `\globaldefs` against the accumulated `\global` prefix
before the assignment case runs. Section 1225 then collects the input with
`read_toks` and commits `define(p,call,cur_val)` at that already-selected
scope. Section 1269 backs up the pending `\afterassignment` token only after
the definition has committed.

Canonical command scanning now carries that effective scope in the typed read
request. Replay installs the parameterless macro locally or globally from that
field, and the observer publishes its exact `end_match`-plus-replacement
meaning mutation before publishing the afterassignment replay-level push.
Focused units cover explicit prefixes, both signs of `\globaldefs`, group
restoration, exact observed meaning, event ordering, and replay. The committed
`main-control/read-mutation-order` microfixture pins the global mutation.

TeX82 §1257's earlier `new_font` half remains owned by
`umber2-johp.142`: its provisional null-font definition is observed during
the command-owned scan, before the filename and size operand. The focused
`font_definition_scanner_defines_the_null_font_before_scanning_operands` unit
pins that order and value. A fresh optimized exhaustive comparison on this
branch reported zero divergences for both `tex82/document-plain-v1` (which
contains the original font-definition sites) and
`tex82/document-story-v1`; the unchanged Gentle front retained its exact
61-divergence, 24-root-site signature.
