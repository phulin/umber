# Main-control responsibility boundaries

`MainControl` remains the one executor and keeps its command, mode, episode,
observation, and settlement state. Private sibling modules group methods by the
authority they use; they do not own a second command loop or durable state.

Resource resolution asks the host provider for a typed answer, installs it in
the existing command capabilities, and checks the exact requested resource.
Input streams, fonts, and PDF images share this boundary because a resource
miss unwinds the current attempt for checkpoint replay. The methods in
`main_control/resources.rs` retain that order and the existing error mapping.

Root source registration and framing are distinct from resource resolution:
the host has already supplied immutable source bytes. The methods in
`main_control/root_source.rs` keep the one-root rule, editor rebind, startup
string accounting, and root completion policy on `MainControl`.

`main_control/discretionary.rs` applies the live `\discretionary` and `\-`
command family. It opens the semantic group and restricted horizontal level,
validates each completed part, schedules `\aftergroup`, and appends the final
node. Operand scanning still belongs to `tex-command`; group, mode, and node
mutations still use the existing `MainControl` fields and `Universe` context.
The outer episode retains resource replay, diagnostics, and settlement.

The production driver in `main_control.rs` still owns admission, dispatch,
replay, and completion. `command_episode.rs` owns admitted command and typed
cold payload state; `settlement.rs` owns commit and rollback. Moving methods
across these private files must preserve the same `MainControl` methods,
resource errors, and observable terminal, effect, and artifact ordering.

TeX82 code-table assignments scan their character selector, optional equals,
and signed value through one operand helper in `delivery.rs`. Hot `\catcode`
and the cold `\lccode`, `\uccode`, `\sfcode`, `\mathcode`, and `\delcode`
paths then share the value bounds and invalid-code zero recovery in
`code_table.rs`. Their distinct committers remain responsible for scoped
mutation, tracing, and ordered publication. The committed command-semantic
`etex-def-code-profile` case contains all six commands and committed terminal
and log references. The TeX82 `command-transitions-v1` fixture pins a wider
assignment sequence and its output channels. Focused executor tests cover
invalid-value diagnostics, zero recovery, local restoration, and later global
writes without changing reference fixtures.

The selected exact-reference gate is
`UMBER_COMMAND_SEMANTIC_CASE=etex-diagnostics/etex-def-code-profile cargo test -q -p tex-command-stream --test it command_semantic::declared_command_semantic_cases_match -- --exact --ignored --nocapture`.
It fails on both sides of this refactor at the same pre-existing source-framing
line: the reference terminal line 3 and log line 4 contain
`(./etex-def-code-profile.tex )`, which the runner omits. The same fixture was
executed before and after the change with `execute_with_provider`, and complete
actual outputs compared byte for byte: projected commands (568 bytes), all
observations (127,150 bytes), terminal (165 bytes), log (162 bytes), and event
count/status (24 bytes) match; DVI, effects, and diagnostics are empty on both
sides. This establishes refactor equivalence for the case, while the selected
oracle gate remains failing.
