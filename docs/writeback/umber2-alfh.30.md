# `umber2-alfh.30`: Oracle-backed effects channel

The command-semantic `effects` channel now compares a portable projection of
the pinned reference oracle with the same projection of Umber observations.
It never serializes `tex_exec::World` effect records as expected data.

The normalized channel retains numbered-stream `open`, `write`, and `close`
events from the shared `tex_oracle::EffectEvent` schema in observation order,
then appends exact generated-file bytes in bytewise logical-path order.
Terminal/log writes, shipout, termination, and specials remain owned by their
existing terminal, log, status, and DVI channels. An explicitly reviewed
`unsupported` disposition is available only for effects that have no portable
reference projection; it carries a reason, commits no bytes, and produces no
verdict.

Three focused fixtures migrate the retained `tex_exec_io` cases into the
active command-semantic corpus:

- `open-close-without-write` exactly matches TeX82 §§1370 and 1375, including
  ordered effects and three empty generated files.
- `top-open-write-close` observes §§1370, 1373, and 1374. Its ordered events
  match; `umber2-johp.762` pins Umber's missing deferred write bytes.
- `closeout-stream-selectors` observes §§1342, 1370, and 1375.
  `umber2-johp.761` pins missing `\openout` transcript lines and
  `umber2-johp.763` pins files not materialized before a no-shipout exit.

The oracle runner now consumes singleton schema-2 manifests and writes a
sorted `effect-artifacts.txt` inventory beside each observation stream. The
regenerator reads only that pinned stream and those oracle artifacts. Running
the same exact regeneration twice is byte-idempotent.
