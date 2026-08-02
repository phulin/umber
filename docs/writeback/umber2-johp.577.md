# umber2-johp.577 — physical post-break synchronization kern

Authority: TeX82 `tex.web` §§904, 914, and 918.

The physical post-break branch of a through-ligature discretionary ends at a
character synchronization boundary, not merely after consuming the counted
character span. A font kern produced between the final reconstructed
character and the next source character belongs to that branch tail.

Hyphenation now carries the next source character into physical post-span
construction. If reconstructing the final partial ligature does not already
emit a font kern, the shared font-boundary lookup recovers either pdfTeX's
automatic kern or the font's lig/kern-program kern for that exact character
pair. This remains confined to `NodeSequence`'s diagnostic physical channel;
semantic packing and shipout are unchanged. A bounded projection regression
also proves that a trailing physical font kern remains owned after the counted
character span is satisfied.

Guarded format-loaded TRIP advances the gating log mismatch from byte 49889
to byte 50010. The expected 4-point synchronization kern now appears under
the post-break `..|` branch. The actual log SHA-256 changes from
`8ef1af7f870397579eeb788f5fb455a5e1a02c45767b6c65ff0ff82533d08ead` to
`29096fd5c38e5e06c06d8562e40d475adbee85b4e45bdacc43ce0303f0f1e0df`, while
normalized DVI and all 22 command events remain exact. The next discretionary
count/character projection is tracked by `umber2-johp.578`.

The branch's focused tests, guarded TRIP, exhaustive command tracer, and Story
locator pass. The base-wide suite and `scripts/check.sh` are temporarily
blocked by the unrelated zero-fuel `tex-incr` regression and its active
format/lint repair in `umber2-johp.26.4`; `.577` depends on that issue and must
be rebased onto its fix before final gate reruns and closure.
