# `umber2-johp.152` Build-Cache Policy

The repository keeps Cargo incremental compilation enabled. Parallel worktree
targets are independent by default, so coordinators must run
`scripts/build-cache-policy.py --jobs N` before dispatch and reserve 12 GiB per
new job plus 4 GiB for the filesystem.

Low capacity never triggers automatic deletion. The tool's explicit
`--reclaim` mode is limited to the current checkout's validated
`target/debug/incremental` and `target/clippy` directories and refuses while a
Cargo-family process is active in that checkout.
