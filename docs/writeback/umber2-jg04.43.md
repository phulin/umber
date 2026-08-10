# umber2-jg04.43: code-table save-stack projection

The exhaustive canonical tracer is clean. After box-spec accounting advanced,
fresh-cache compatibility TRIP first differed in TeX82 §1334's final
save-stack statistic: the pinned oracle reported `38` positions while Umber
reported `37`. Command, geometry, and normalized-DVI hashes remained exact.

At the pinned Web2C TeX82 32-word high-water on `trip.tex` line 296, the live
stack contains two restore records created by the preceding local `\catcode`
assignments to `J` and `j`. TeX82 §240 places code tables in `eqtb` at
`level_one`; §275's `eq_save` therefore preserves each first local assignment
as a two-word `restore_old_value` record. Umber's `CodeTables` already retained
the two typed records for correct group restoration, but the diagnostic
projection counted only the independent `Env` journal.

The generic projection now includes two words for every live typed code-table
restore record. Focused positive and negative controls cover first local
assignment, reassignment within one local run, a retaining global assignment,
a later local run, and a nested group. This preserves `CodeTables` as the
semantic owner and changes no comparison or normalization.

Fresh-cache TRIP changes from `37` to `41` save-stack positions. The remaining
independent `41`-versus-`38` front is recorded observed-only as
`umber2-jg04.44`; all other exact artifact hashes remain unchanged.
