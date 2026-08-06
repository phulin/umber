# umber2-vgjr.14.4 — streamed bibliography boundaries

`bib-input` now parses XML into one flat bounded projection arena rather than
an recursively owned generic tree. BCF, configuration, and BibLaTeXML consume
borrowed element views. XInclude resolution projects each included document,
preserving path selection, cycle detection, include ordering, and all existing
byte, nesting, node, attribute, text, and include limits.

Bibliography sessions validate configuration directly and retain only selected
input paths; they no longer construct and discard `ConfigurationFile`. Public
typed parsing, validation, limits, and compatibility types remain available.

`bib-output` has one router-owned bounded sink for every serializer. The sink
owns bounded UTF-8 work, newline conversion, legacy encoding, exact byte
limits, and final `GeneratedFile` construction. The router no longer allocates
a second selected-section graph. Public serializer and `OutputPlan` surfaces,
Unicode compatibility checks, filenames, and bytes are unchanged.

Production Rust adds 545 lines and deletes 558, a net deletion of 13 lines.
No fixtures or generated sources changed, and no linked discovery was needed.

Focused input/output/engine execution passed under `MemoryMax=512M`: 72 engine
unit tests, 356 separately named compatibility tests, 23 input tests, and 12
output tests passed; 940 declared ignores remained. The complete routine
workspace passed after an uncapped `--no-run` build under `MemoryMax=1G`.
The exact WASM test scope built uncapped; its Node test gate and packaged
TeX-to-bibliography-to-TeX fixture passed under 1 GiB. Firefox could not launch
in this host because `geckodriver` was killed before session creation; no test
body ran. `scripts/check.sh` passed all four gates under the 1 GiB cgroup.
