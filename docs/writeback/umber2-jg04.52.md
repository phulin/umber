# umber2-jg04.52: Web2C SyncTeX Parameter Occupancy

The exhaustive canonical tracer is clean. After the allocator statistics
became exact, official e-TRIP first differed in the final control-sequence row:
the pinned oracle reported `409` multiletter names and Umber reported `408`.
Compatibility TRIP and normalized e-TRIP DVI were already exact.

A hardware watch on the pinned Web2C e-TeX `cs_count` proved that the loaded
format starts with `408` entries and increments to `409` at e-TRIP line 156.
The stack is `id_lookup`, `get_next`, `scan_general_text`, `the_toks`, and
`scan_toks`; it creates the deliberately undefined `\splittopmarks` name in
the `\unexpanded` primitive inventory. Umber made the same runtime entry but
started its loaded run at `407`. A complete pinned hash spelling dump and the
typed primitive inventory reduced the format delta to the sole absent
`synctex` parameter.

TeX82 §§256 and 259 establish permanent hash occupancy, §1309 serializes the
occupied table, and §1334 reports its count. The pinned TeX Live Web2C
[54/SyncTeX] section installs `\synctex` as an `assign_int` primitive. That
change layer belongs to the official e-TeX oracle build but not the TeX82
compatibility oracle's change stack.

The generic fix adds `\synctex` to the extended integer-parameter catalogue,
with the ordinary zero default, grouping, assignment, and format persistence.
The e-TeX static-pool origin compensates for the newly typed spelling so the
pinned `init_prim` coordinates remain exact. String-pool profile version 11
and prepared-format producer contract 14 reject cached images built before
that vocabulary. Focused positive controls prove the meaning, default, and
format round trip; the negative control proves the TeX82 profile still omits
the name.

Exact compatibility TRIP and the official two-phase e-TRIP artifact gate both
pass. The official transcript, log, generated output, DVItype-derived DVI
identity, memory statistics, and control-sequence count now compare under the
unchanged normalization contract.
