# umber2-e51h.68.5 — whatsit traversal through shutdown

TeX82 §§1362–1378 complete the base-whatsit lifecycle after construction:
passive list consumers preserve order and dimensions, DVI traversal emits
special bytes at their hlist position, writes expand through the shared
`write_out` path, language nodes freeze normalized state, and termination
closes every still-open numbered output stream before its terminal event.

`mixed_hlist_traversal_and_special_byte_lengths_match_tex82` places 255- and
256-byte specials among ordinary kern nodes in a real hlist traversal. It
proves list order, exact payload bytes, and §1368's `xxx1`/`xxx4` boundary.
The existing passive-consumer tests cover §§1362–1367's language, page-builder,
and vertical-break visits without introducing effects or dimensions.

`immediate_and_deferred_writes_preserve_expanded_tokens_exactly` compares the
owned semantic token lists produced by §§1369–1375 for the two execution
times. Existing effect-order, stopper-retirement, retry, leader-suppression,
and selector tests retain the rest of the `write_out` and `out_what` contract.

`language_normalization_and_same_language_append_boundaries_match_tex82` and
`setlanguage_illegal_mode_recovers_without_scan_or_append` preserve the
catalogued names for §§1376–1377's normalization, unconditional repetition,
captured minima, and mode-first recovery. The shutdown test proves §1378
closes only live numbered streams, in slot order, before engine termination.
