# umber2-jg04.38: TeX82 box-error direct mutation

The exhaustive canonical command tracer is clean. After the frozen graph panic
was removed, fresh-cache TRIP first differed when group exit reported a retained
local value for insertion register 100. The source of that state was the
page-builder recovery path for an invalid insertion hbox.

TeX82 §993's `box_error` displays and flushes the rejected list, then performs
the literal assignment `box(n):=null`. It does not call §275's `eq_define`, so
it creates no local save-stack entry for §283 to restore or retain. Umber now
routes this operation through the existing same-level box mutation seam instead
of the ordinary local assignment barrier.

The focused regression runs §993 recovery inside a group, installs a later
global box, and proves that group exit has no retained-value trace. Its negative
control performs an ordinary local clear followed by the same global write and
requires the §283 trace, preventing the fix from weakening assignment behavior.

At commit `01c802275`, focused and complete `tex-exec` tests pass. Fresh-cache
TRIP no longer differs at the restore trace: its next first difference is the
final string/statistics block at log byte 182641, filed observed-only as
`umber2-jg04.39`. Official e-TRIP and the routine workspace suite pass; all
repository checks pass after the test-only lint correction in the writeback
commit.
