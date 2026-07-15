# pdfTeX generated fonts and microtypography

Status: implementation contract for pdfTeX 1.40.27 font construction,
expansion, and character protrusion.

## Upstream boundary

Behavior is pinned to `pdftex.web` at TeX Live source commit
`1664cf0ab3f6ce3b80db649bc6723f54ab12016c`, the same pdfTeX 1.40.27
boundary as `pdftex_primitives.md`. The owning source routines are
`letter_space_font`, `copy_font_info`, `read_expand_font`, `try_break`,
`hpack`, `do_subst_font`, `total_pw`, and `post_line_break`.

The implementation must preserve these observable thresholds:

- positive `\pdfadjustspacing` substitutes expanded fonts while packing
  finalized paragraph lines; values greater than one also include expansion
  capacity in breakpoint feasibility;
- positive `\pdfprotrudechars` inserts margin-kern nodes into finalized
  paragraph lines; values greater than one also include protrusion in
  breakpoint feasibility;
- `\efcode` is clamped to `0..=1000`, while `\lpcode` and `\rpcode` are
  clamped to `-1000..=1000` by the existing per-font code-table boundary.

## Ownership

`tex-fonts` owns immutable backend-neutral font derivations. A generated font
records its derivation from a semantic base font: copied, letterspaced, or
expanded by a signed thousandths ratio. The derivation participates in the
font's content identity even when the underlying TFM bytes are shared.
Letterspacing derives character widths and an output placement offset from
the base em; expansion derives character widths, italic corrections, and
font-kern amounts by TeX's rounded `(1000 + ratio) / 1000` scaling. Height,
depth, and font parameters retain pdfTeX's base values.

`tex-state` owns live `FontId` allocation, control-sequence identifiers,
per-font code tables, expansion specifications, generated-font links, and
the mapping from `(base semantic identity, signed expansion ratio)` to a live
font. Generated fonts are immutable after publication. Their derivation,
expansion specification, code tables, semantic hash, snapshot behavior, and
format DTO are all state, never a host cache.

The expansion command validates stretch `0..=1000`, shrink `0..=500`, and
step `1..=100`, rounds both limits down to a step, and rejects two zero
limits. Repeating the command is accepted only with identical parameters.
Expanded and letterspaced fonts retain the source-level restrictions on
copying and re-expansion. Generated fonts are allocated in source order so
later ordinary font numbers and diagnostics remain deterministic.

`tex-typeset` owns pure expansion/protrusion calculations. Its immutable font
view exposes character metrics, font-kern lookup, expansion specifications,
generated-font ancestry, and pdfTeX character codes. The line breaker carries
natural width plus normal-order font stretch and shrink. It does not allocate
fonts or mutate `Universe`.

`tex-exec` owns the mutation boundary after a winning line has been selected.
It asks the pure pack planner for the signed line expansion ratio, interns only
the derived fonts actually selected by that line, substitutes glyph and
ligature font ids, adjusts intervening font kerns, inserts margin-kern nodes,
then performs the ordinary final hpack. This matches pdfTeX's lazy derived-font
creation without moving state mutation into a typesetting kernel.

## Line-breaking and final material

All arithmetic uses `tex-arith` scaled operations and pdfTeX's signed rounding.
For each expandable byte glyph, stretch and shrink capacity are the endpoint
metric difference multiplied by `efcode / 1000`. Font kern capacity is the
corresponding endpoint-kern difference scaled by the left glyph's `efcode`.
One paragraph may use only one nonzero expansion step and consistent nonzero
stretch/shrink limits, matching pdfTeX's paragraph diagnostics.

When breakpoint-aware adjustment is enabled, the candidate shortfall first
includes left and right protrusion where enabled, then consumes available font
stretch or shrink before glue badness is computed. Discretionary pre/post and
replacement lists follow the same breakpoint-local width rules as ordinary
line width.

For a finalized line, the pack planner calculates a signed expansion ratio
only when normal-order glue applies and matching font capacity is nonzero.
Each glyph selects the nearest legal derived-font step after applying its
`efcode`; zero `efcode` retains the base font. Derived font substitution also
updates normal font kerns and discretionary pre/post glyphs.

Protrusion finds the first and last eligible glyphs using pdfTeX's nested hlist
and discretionary rules. A left margin kern precedes the glyph material and a
right margin kern follows it before `\rightskip`; each amount is the negative
of `round(em * code / 1000)`. Margin kerns retain side, source font, and byte
character so expansion-aware repacking, diagnostics, enquiries, snapshots,
and output lowering do not infer provenance from neighbors.

`\leftmarginkern` and `\rightmarginkern` scan a box register, require a
non-empty hbox, skip the corresponding line skip glue and other pdfTeX
skipable nodes, and expand to the stored signed dimension or `0pt`.

## Committed artifacts and PDF identity

Committed page artifacts distinguish the semantic font resource used by each
glyph. A generated-font resource carries a detached derivation identity in
addition to its classic/OpenType program identities. Copy and letterspace
identity cannot collapse into the base resource merely because TFM or OpenType
bytes match; expanded ratios likewise remain distinct resources.

The output layer remains driver neutral. DVI consumes the finalized metric and
font number in the ordinary way. PDF finalization maps detached generated-font
identity into resource dictionaries and delegates all final bytes, streams,
indirect objects, cross references, and trailer framing to the canonical
vendored `pdf_writer` adapter. Font map lookup, embedding, subsetting, and
ToUnicode remain owned by issue 17; this contract introduces no dependency
from generated-font state back to that later backend.

## Validation

Focused pinned-oracle fixtures cover successful construction, clamp and error
diagnostics, repeated expansion setup, mixed expansion rejection, discrete
step selection, line breaks, signed protrusion, margin enquiries, box dumps,
snapshot/rollback, format round trips, and generated font names. Hermetic
tests cover immutable derivation arithmetic, semantic identity, artifact
round trips, and unchanged DVI behavior when microtypography is disabled.

The completion gate is the focused crate tests followed by `scripts/check.sh`
and `scripts/check-and-test.sh`.
