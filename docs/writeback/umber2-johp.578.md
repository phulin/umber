# umber2-johp.578 — physical discretionary synchronization projection

Authority: TeX82 `tex.web` §§914–918.

An automatic discretionary's replacement count is the number of linked
major-branch nodes produced before the major and post-break reconstitutions
reach the same source-character boundary. It is not the number of source
characters retained inside a structured ligature. A `CA` ligature can
therefore be one replacement node with a one-character `A` post-break branch,
while another two-character ligature can require three replacement nodes and
a longer post-break branch before synchronization.

Hyphenation now reconstitutes the complete post-break suffix in the diagnostic
physical channel and aligns its source-character boundaries with those of the
major branch. The first shared boundary determines both the physical
replacement count and the exact post-break projection. Semantic ligatures,
line breaking, packing, and shipout remain unchanged. Bounded regressions pin
both immediate one-node synchronization and the earlier three-node branch
with its trailing synchronization kern.

Guarded format-loaded TRIP advances the gating log mismatch from byte 50010
to byte 50049. The expected `replacing 1`, pre-break `C` and hyphen, and
post-break `A` are now exact. The actual log SHA-256 changes from
`29096fd5c38e5e06c06d8562e40d475adbee85b4e45bdacc43ce0303f0f1e0df` to
`a8e1e48b571cfcdb7ddf1306a68470f24ca340aeca600b66253470a19a50647b`, while
normalized DVI and all 22 command events remain exact. The next character
diagnostic rendering front is tracked by `umber2-johp.579`.

The two bounded physical-branch projection regressions pass. The guarded TRIP
run produced the evidence above and is expected to fail at the newly observed
front. The exhaustive command-stream tracer, Story locator, full native suite,
and `scripts/check.sh` remain pending because a repository-wide OOM incident
required every Cargo and Rust process to stop; rerun those commands with
`CARGO_BUILD_JOBS=6` after the shared memory pressure clears.
