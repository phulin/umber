# umber2-vgjr.17 — VFS transaction and maps closeout

The exact combined implementation tree is
`b2b404fc32b04c0926e99750eab5c30dbff21b45`. All three children are closed
with substantive writeback. The durable roadmap in `docs/umber_vfs.md` and
`docs/persistent_compile_sessions.md` keeps TeX--bibliography--TeX scheduling,
convergence, and complete-set assembly in `LatexProjectSession`. One
`ProjectWorkspace`-owned `GeneratedTransaction` is the only pending
publication authority. Its candidate map is private and separate from the
durable copy-on-write generation, whose only maps are the private, shape-safe
`UserFiles`, `ResolvedFiles`, and `GeneratedFiles` types.

`VirtualFs`, `BuildPlan`, `BuildTransaction`, `StageTransaction`, declared
replacement and invalidation, producer/build/stage identities, public
`LayerKind`, `FileLayer`, and `LayeredFileStorage`, and their test-only
multistage and generic-construction fixtures are deleted. Caller inventory
proved every production VFS build used one stage before removal. Retained Rust
and WASM session adapters preserve their attempt, resource, output, retry, and
rollback contracts; no compatibility shim, pass-plan wire type, pending layer,
or second storage authority remains.

Commits `78706f0e5` and `b2b404fc3` add 385 and delete 949 production Rust
lines (net -564), add 321 and delete 629 Rust test lines (net -308), and add 86
and delete 98 documentation/guidance lines (net -12). Implementation therefore
adds 792 and deletes 1,676 authored lines, a net deletion of 884. Including the
roadmap commit `c0af1fa22`, the complete program adds 706 and deletes 1,578
Rust lines (net -872) and adds 179 and deletes 111 documentation/guidance lines
(net +68), for 885 additions, 1,689 deletions, and 804 lines of total net
deletion. Declarative/generated records and binary assets are unchanged. The
implementation result is inside the 750-1,000-line forecast, so no reduction
shortfall issue is required.

Fresh closeout verification compiled the focused `umber-vfs`, `bib-input`, and
`umber` selection uncapped with `--no-run`, then passed it under
`MemoryMax=512M` with a 383,340,544-byte peak and no cgroup pressure or OOM
events. The complete native workspace compiled uncapped with `--no-run`, then
passed under `MemoryMax=1G` with a 441,454,592-byte peak and no memory events.
The wasm32 target compiled uncapped as tests and passed `cargo check` under 1
GiB at a 55,394,304-byte peak. The optimized package rebuilt under 1 GiB at an
828,149,760-byte peak; all 89 authored Node tests passed at a 123,224,064-byte
peak, and its packaged TeX--bibliography--TeX project lifecycle passed at a
101,203,968-byte peak. Every valid run used a finite timeout. The browser
package runner reached its host prerequisite check but `/usr/bin/google-chrome`
was absent; the independent browser-driver environment issue remains
`umber2-5zie`. `CARGO_BUILD_JOBS=1 scripts/check.sh` passed all four gates
under `MemoryMax=1G` at a 115,331,072-byte peak with no memory events.

One initial focused wrapper invoked the host's older Cargo and rejected
workspace resolver 3 before compilation. It is not test evidence; rerunning
with the repository toolchain produced the successful capped receipt above. A
later combined browser-package attempt reached the 1 GiB cap during a redundant
package rebuild without an OOM and then reported the missing Chrome binary;
the isolated rebuild and Node runs above are the valid measured evidence.
