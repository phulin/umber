# umber2-vgjr.12.1 — typed closed-case and Git inventory contract

`test-support::closed_case` now defines normalized case and publication identities, ordered tracked input/expected-output/metadata roles, optional compatible SHA-256 pins, pass/xfail status with durable reasons, profiles, exact source closure, and fixturegen-only publication metadata. Validation composes with the existing selected-checkout `ClosedCase` authority and preserves `closed-case-v1` paths, membership, declared order, byte hashes, traversal rejection, and hash-optional local fixture edits.

Validated cases can stage byte-exact closed candidates, but expose no publication operation. Fixturegen remains the sole authority mutator and now delegates its cohort candidate validation to the shared staged-case contract; its existing plan digest, transaction markers, rollback, retry, and atomic rename machinery is unchanged. `corpus-manifest` and its dependency-free graph were not modified.

Focused proof covers typed parity, missing/extra/hash/order/closure failures, traversal, local edits, staged compatibility, Git authority, and all 33 fixturegen tests. The final tree also passed the complete native workspace suite and `scripts/check.sh` under the required cgroup limits.

Implementation metrics before this writeback: 637 added and 77 removed authored Rust lines (560 net), including focused contract tests and replacing 72 lines of duplicate fixturegen staged-inventory validation. The downstream V2 and consumer-migration issues own adoption and further deletion.
