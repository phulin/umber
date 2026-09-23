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
