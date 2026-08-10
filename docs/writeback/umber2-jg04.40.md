# umber2-jg04.40: TeX82 main-memory extent

The exhaustive canonical tracer remained clean: zero semantic and geometry
divergences. Fresh-cache compatibility TRIP first differed only in §1334's
final main-memory row, reporting `2440` words where the pinned oracle reports
`3556`; command, geometry, and normalized-DVI hashes were already exact.

The pinned Web2C TeX82 executable was stopped at `close_files_and_terminate`.
It held `lo_mem_max=3020`, `hi_mem_min=249465`, and `mem_end=249999`, so TeX82
§1334's inclusive coordinate formula is exactly `3021 + 535 = 3556`. Sections
125--127 establish the split allocator and its persistent coordinates; §127
grows low memory in 1000-word blocks; §§133--157, 683, and 790 declare the
canonical node sizes; and §1334 reports allocator extent rather than live
`var_used`/`dyn_used`.

The generic fix projects the reachable frozen-format closure back onto those
canonical node sizes. Character and ligature-source nodes occupy §125's
one-word arena, while variable-size nodes and reachable glue specifications
occupy the low arena. Diagnostic-only replacement children do not count twice
as live nodes, but the largest directly mutated physical branch and four
engine scratch words remain in the high allocator coordinate after release.
Focused controls show that 501 two-word penalty nodes cross one low-memory
growth boundary while 501 character nodes do not, and that the high coordinate
survives rollback.

Fresh-cache exact TRIP now matches the complete `3556/250000` row and advances
to the independent multiletter-control-sequence statistic (`372` expected,
`388` actual). That front is recorded observed-only as `umber2-jg04.41`.
