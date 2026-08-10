# umber2-jg04.44: null token-list restore-zero projection

The exhaustive canonical tracer is clean. After code-table restore records
became visible, fresh-cache compatibility TRIP first differed in TeX82 §1334's
final save-stack statistic: the pinned oracle reported `38` positions while
Umber reported `41`. Command, geometry, and normalized-DVI hashes remained
exact.

At the pinned Web2C TeX82 high-water, raw save-stack slot 24 is the one-word
`restore_zero` record for `every_display_loc`. TeX82 §240 initializes all
token-list parameters to `undefined_control_sequence` at `level_zero`.
Sections 275--276 therefore preserve a first local assignment to such a cell
with one word, not the two words used by `restore_old_value`.

Umber's `OptionalTokenListIdCodec` already preserves that distinction: zero is
the null pointer and one is a defined empty token list. The generic save-stack
projection now classifies a typed `TokParam` old word of zero as
`restore_zero`, alongside an undefined meaning. Focused controls prove the
null outer value occupies one restore word while defined-empty remains the
two-word negative control.

Fresh-cache TRIP advances from `41` to `40` save-stack positions. The remaining
independent `40`-versus-`38` front is recorded observed-only as
`umber2-jg04.45`; all other exact artifact hashes remain unchanged.
