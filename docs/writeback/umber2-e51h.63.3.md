# Math-list Pass and Recovery Coverage

Issue `umber2-e51h.63.3` audits TeX82's missing-character and undefined-family
recovery together with the two Appendix G math-list passes.

The active `mlist_passes_cover_all_styles_bins_nonscript_spacing_and_penalties`
test covers TeX82 §§728--733 and §§761--767. It exercises all eight
cramped/uncramped styles, choice selection, rules 5 and 6 binary-operator
transitions, script-style `\nonscript` suppression, and rule 21's bin/rel
penalties. The existing exhaustive inter-noad spacing table remains covered by
`tex82_second_pass_spacing_delimiter_penalty_matrix`.

TeX82 §§722--724 and §755 require two execution-visible outcomes that the
current pure `tex-typeset` conversion boundary cannot report: canonical
`char_warning` while omitting only a missing character, and an error that
deletes a formula when its selected math family is undefined. The exact tests
`missing_math_character_reports_canonical_warning_and_omits_only_character`
and `undefined_math_family_reports_error_and_recovers` are retained as ignored
specifications. Issue `umber2-e51h.63.7` tracks a typed conversion-event and
recovery boundary that can enable them without coupling the pure kernel to the
execution printer.
