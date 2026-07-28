# `umber2-johp.190`: Committed Open/Close Effect Observations

TeX82 §1342 owns the `write_open[0..17]` table and keeps fallback selectors 16
and 17 permanently closed. Sections 1373–1374 execute an open or close in
`out_what`; §1375's `\immediate` is only one route into that same operation.
Open and close observations therefore come from the committed
`tex_state::EffectRecord` delta, not from the scanned `\immediate` command.

Immediate effects are read from the live applied-step delta. Deferred effects
are created and committed during shipout, whose eager transaction drains the
World prefix before main control resumes, so the shipout receipt carries that
same committed delta across the boundary. Both paths now publish the same
ordered open/close records exactly once and before the page's shipout
observation.

`World::close_out` records a close only when the selected numbered stream is
open. Never-open streams and normalized selectors 16 and 17 remain silent.
The close observation remains `stream:N\0` with no remembered target, preserving
the separate oracle-payload correction from `umber2-johp.189`. Deferred
`CloseOut { slot: None }` nodes also no longer reserve a nonexistent page-effect
anchor.

Three table-focused replay tests cover immediate and deferred open/close,
closed and normalized no-ops, ordering, and exactly-once behavior. The
hermetic `page-output/open-close-effect-observation` command-semantic
microfixture commits the deferred path, and the TeX82 property catalogue links
both test layers to §§1342 and 1373–1375. Automated tracing remained restricted
to committed microfixtures.

## Validation

The first native run could select only 44 of 48 test binaries because this
worktree lacked the gitignored conformance inputs, DVI oracles, and plain-TeX
font metrics. After copying those exact declared assets from the primary
checkout, the final native verdict was:

```text
run-native-tests: VERDICT: PASS - 33 packages, 48/48 test binaries, 3971 passed, 0 failed, 941 ignored; TeX82 property catalogue: 938 reviewed, 442 deferred; 100 covered, 51 gap; deferred tiers: 0 of 6 passed on this tree
```

The focused command-semantic suite passed all seven harness tests, and the
combined `tex-state`/`tex-exec` suite passed all 910 library tests.
