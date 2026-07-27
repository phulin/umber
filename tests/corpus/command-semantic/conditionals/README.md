# TeX82 Conditional Semantic Minifixtures

These hand-authored inputs are the hermetic semantic tier for the eight
conditionals properties that are not blocked by the box-register selector gap.
They run through `tex_exec::CanonicalMainControl` with the TeX82 INITEX exact
8-bit profile and compare short, exact projections of committed
`tex_command::CommandObservation` records. They do not invoke TeX, load a
format, read the long-document trace registry, or copy expected bytes from a
reference run.

Canonical provenance is the `tex.web` identity pinned by
`tests/tex82-oracle-manifest.txt`. Expected behavior comes from the numbered
sections below; the retired Umber lexer/expander is not an oracle.

| Input                    | Property                                   | `tex.web` sections | Exact semantic projection                                               |
| ------------------------ | ------------------------------------------ | ------------------ | ----------------------------------------------------------------------- |
| `classification.tex`     | `tex82.conditionals.classification`        | §§210, 487, 491    | `if_test` operands 0–16 and `fi_or_else` operands 2–4                   |
| `stack-lifecycle.tex`    | `tex82.conditionals.stack-lifecycle`       | §§489, 495–497     | nested push, branch, limit, and LIFO pop                                |
| `skipped-text.tex`       | `tex82.conditionals.skipped-text`          | §§493–494          | skipping-status bracket, nested raw skip, and selected assignment       |
| `branch-delimiters.tex`  | `tex82.conditionals.branch-and-delimiters` | §§498–500, 509–510 | positive/negative `ifcase`, boolean false limb, and delimiter stops     |
| `predicate-dispatch.tex` | `tex82.conditionals.predicate-dispatch`    | §501               | vertical/horizontal/math/inner, stream, and constant predicate branches |
| `ordered-relations.tex`  | `tex82.conditionals.ordered-relations`     | §503               | all three integer/dimension relations and missing-relation recovery     |
| `odd-integer.tex`        | `tex82.conditionals.odd-integer`           | §504               | signed even/odd scanner results and branches                            |
| `token-predicates.tex`   | `tex82.conditionals.token-predicates`      | §§506–508          | expanded character/category and raw meaning/macro comparisons           |

TeX82 §505 is deliberately absent. Its out-of-range box-register selector is
tracked separately and must not be hidden by a reduced expectation here.
