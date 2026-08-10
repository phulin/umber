# umber2-jg04.49: Pre-restore Main-Memory High Water

After the official e-TRIP string-pool rows became exact, the exhaustive
canonical tracer remained clean: zero projected semantic divergences and exact
normalized DVI. The first compared artifact difference was the final memory
row, `3317` words in the pinned Web2C oracle and `1342` in Umber.

The pinned e-TeX executable at `close_files_and_terminate` held
`lo_mem_max=2020`, `hi_mem_min=248704`, and `mem_end=249999`. TeX82 §1334's
inclusive allocator extent is therefore `2021 + 1296 = 3317`. TeX82 §§125--127
make those low/high coordinates persistent high-water state, while §283 frees
group-scoped values during `unsave`. The e-TeX sparse-array implementation
likewise creates variable-size nodes during register assignment and deletes
unreferenced default elements during restoration; changes [49.1236] and
[50.1311] establish that those nodes use the ordinary allocator and that the
six register roots belong to the dumped format state.

Umber previously projected main memory only after restoration. The generic fix
samples the reachable typed closure immediately before both group-exit paths,
then performs the existing restore. A focused positive control binds a
600-word token list locally and proves that §1334's high coordinate survives
the group exit. Its negative control interns the same host-store value without
making it reachable and proves that representation history does not count.

Exact compatibility TRIP remains green. Official e-TRIP advances from `1342`
to `1999` words, leaving the separate low-memory sparse/extended-state extent
front recorded observed-only as `umber2-jg04.50`.
