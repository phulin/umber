# Compatibility surface removal

The current implementation has one owner for each engine operation and one
current representation for each host boundary. The September 2026 follow-up
explicitly permits breaking Rust, CLI, cache, and wire APIs to remove obsolete
Umber compatibility surfaces. New code must call the current owner directly;
it must not preserve an old-shaped forwarding facade, fallback decoder, or
duplicate execution path merely to keep an earlier Umber interface working.

This is an implementation cleanup, not a change to TeX's language contract.
TeX82, e-TeX, pdfTeX, classic font mappings, LaTeX profiles, complete-job and
fragment semantics, diagnostics, resource ordering, effects, and current
output bytes retain their independent oracle and product tests. Historical
documents and pinned reference fixtures remain evidence. Bibliography is
outside this pass.

The current owners are:

| Concern                                                    | Current authority                                                                                                                                               | Migration boundary                                                                                                                                               |
| ---------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Raw and expanded command delivery, scanners, macro calls   | `tex-command::CommandProcessor` and its caller-owned typed slots                                                                                                | Remove owned-return and prototype shims after their test callers use the canonical delivery operations.                                                          |
| Main-control execution and resource suspension             | `tex-exec::MainControl::advance` and the canonical step runner                                                                                                  | Remove the old step/error translation; preserve exact typed need, failure, and completion behavior.                                                              |
| Cold and incremental revision lifecycle                    | `tex-incr::Session` candidate, resolver, and publication APIs                                                                                                   | Remove spelling aliases and uncalled generic trace compatibility exports; preserve accepted/rejected revision behavior.                                          |
| Resource resolution                                        | Borrowed command input admission, the typed need/result protocol, and host-owned replay                                                                         | Consolidate the retained host adapter only with its native, virtual, format, and incremental callers and their replay tests.                                     |
| Reference execution and fixture authority                  | `fixturegen::reference` for process execution and publication; `test-support::dvi` for byte comparison; `parity-harness` for conformance composition and triage | Retire the `refexec` compatibility facade and command only after every script and parity caller has a current owner. Fixture regeneration remains transactional. |
| Node/state, artifact/format/output, and host/WASM adapters | Their respective production owners and current schema                                                                                                           | Remove obsolete adapters with their callers; no import of old Umber-owned data is required.                                                                      |

Removal is complete only when repository callers no longer use the old
surface, active tests exercise the replacement contract rather than a retained
shim, and current documentation names the new entry point. A historical test
that deliberately rejects obsolete data may remain when it protects the
current parser's rejection boundary; a decoder or fallback that accepts that
data may not. Any scoped gate result must identify its exact command and
revision. The native combined gate, quality gates, relevant selected feature
and platform checks, and reference-channel tests establish the resulting
behavior; no fixture is rewritten from Umber output to make a migration pass.
