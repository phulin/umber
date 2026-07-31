# umber2-johp.391 — packed-box diagnostic selector

Authority: TeX82 §§54, 245, 660, 663, and 674.

The overfull, underfull, loose, and tight packed-box headline and abbreviated
horizontal-list display use the live print selector. In batch mode they reach
the transcript only; in the other interaction modes they reach both terminal
and transcript. The following `begin_diagnostic` box dump retains its own
§245 selector transition and history update.

The implementation keeps the headline and dump in that order and does not
defer either across a later interaction-mode transition. Focused positive and
negative controls cover batch log-only routing and nonstop terminal/log
identity with online tracing.
