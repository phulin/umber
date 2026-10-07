# Cold command failures

Command processing returns the actual scanner value or completion. A successful
unit-valued expansion has no result payload. Its return representation should
therefore contain only the success sentinel, rather than a stack slot sized for
an error union.

`CommandError` is the owning pointer to one cold `CommandFailure` record. That
record contains its typed `CommandErrorKind` and optional original token origin.
Construction occurs only where a real failure is raised. Successful scans,
expansion, input conservation and group transitions allocate no failure record
and carry no failure payload through their frames.

The kind retains the actual diagnostic facts: fuel evidence, invariant caller,
filenames, resource enquiry, failure cause, primitive identity or fatal value.
Owned data belongs directly to the cold record. A typed resource-failure packet
keeps its existing allocation because that same need/cause owner transfers
through command, execution and session boundaries. It is not unpacked and
reboxed at each boundary.

Source annotation writes the first origin into the existing failure record.
There is no recursive annotation variant or separately allocated annotation
wrapper. Missing-resource needs remain unannotated and retain their exact typed
identity for full-checkpoint replay. This changes neither semantic input nor the
TeX group stack; no failure owner enters a format or checkpoint.

Consumers inspect the actual kind or move its real payload into the executor's
existing typed error. Ordinary command failures transfer the original allocation
to execution. The previous enum-valued command-error API and its constructors
are deleted; consumers use the pointer owner and actual kind directly.

Verify the release ABI rather than assuming that boxing a payload suffices:
unit-valued calls must return the pointer-sized success/error value without an
aggregate return buffer. Check failure kind, caller, first-origin annotation,
fuel cuts, missing resources and fatal propagation with existing semantic and
replay tests. Measure frozen binaries on identical macro, scanner and original
book workloads. Smaller source or frames alone do not establish a speedup.
