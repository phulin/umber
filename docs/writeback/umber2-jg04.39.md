# umber2-jg04.39: TeX82 string-pool lifecycle

The exhaustive canonical tracer is clean. Fresh-cache compatibility TRIP first
differed only in §1334's final string-pool row: expected `44/13626` strings and
`261/15296` characters, while Umber reported `43/13627` and `264/15299`.
Command, geometry, and normalized-DVI hashes were already exact.

The pinned Web2C TeX82 oracle was stopped at `storefmtfile`. Immediately before
TeX82 §1328 it held `str_ptr=1373` and `pool_ptr=24671`; after retaining
`␣(preloaded format=trip 2026.7.9)` it held `1374/24704`. The open box denotes
the leading space. Dumping strings
1349--1373 proved the complete construction tail: startup component, requested
and resolved input names, transcript name, 20 retained input strings, and the
format identifier. TeX82 §§1309--1310 serialize and restore those pointers;
§1334 uses the restored coordinates as its capacity baseline.

The generic fix makes pool ownership independent of semantic interning.
TeX82 §§341/372's direct-character namespace and §1215's fixed `inaccessible`
slot allocate nothing. Startup names follow §§516--537, Web2C
`slow_make_string` recycles across INITEX as well as loaded jobs, §1252 retains
each physical active/null font identifier, and §1328 retains the identifier.
Web2C tex.ch [49.1260] also forbids flushing a recycled font filename. The
TeX82 §§47/50/226 static-plus-primitive profile now compensates only for the
typed registry's 33-byte spelling projection.

Focused controls prove duplicate `FONT?` allocations remain physical while
fixed/internal and one-scalar names do not allocate, a multiletter hash name
still does, and publication does not allocate after §1328. Fresh-cache TRIP is
exact through the full string row and advances to the independent main-memory
row (`3556` expected, `2440` actual), filed observed-only as
`umber2-jg04.40`.
