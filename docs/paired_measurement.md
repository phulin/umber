# Paired executable measurements

[`scripts/paired-measure.py`](../scripts/paired-measure.py) compares two
already-built executables on one host. It is an evidence runner, not a build
driver: it invokes the executable passed by `--baseline-binary` or
`--candidate-binary` directly and has no Cargo integration. Build commands,
format construction, fixture staging, and cache preparation must finish before
this runner starts.

The runner requires two clean Git worktrees and a full 40-character revision
for each side. It resolves and records each worktree's `HEAD` and tree object,
checks that `git status --porcelain=v1 --untracked-files=all` is empty, and
hashes the executable before the first process. A revision mismatch or dirty
worktree fails before any executable is started. It never checks out, resets,
rebases, or modifies either worktree.

## Evidence lanes

Every manifest and invocation names one lane. The lane is recorded in every
receipt and is not inferred from a binary name.

`release-unobserved` requires `profile` to be `release`, `instrumented` to be
`false`, and no feature name containing `profil`. It is the lane for production
latency claims. The binary must have been built separately with the ordinary
release feature set. The runner cannot prove how an arbitrary binary was
compiled, so the manifest's build identity is part of the evidence contract.
Optional `build` and `toolchain` objects are copied into the receipt; populate
them with the build command's target, compiler versions, and feature
resolution.

`profiling-structural` requires `instrumented` to be `true` and a profiling
feature name. It is suitable for allocator counts, semantic counters, fixed
operation counts, and profiler attribution. Its timing is diagnostic and must
not be presented as production release timing.

The standalone `tex-command` and `tex-exec` benchmark manifests enable their
`profiling` features. Run them under `profiling-structural`, even when Cargo's
profile is named `release`. Run the root `umber` binary under
`release-unobserved` only after building it without profiling instrumentation.
For the release command-core matrix, pass `--observer disabled`; observer
enabled/both runs belong to the structural lane.

## Manifest

The manifest is JSON and has schema `1`. Input paths are absolute so changing
the runner's current directory cannot change the workload. Each input is
hashed before the first warm-up; an optional `sha256` field pins the expected
bytes. Each declared output is relative to the fresh sample directory and is
hashed after the process exits.

The semantic channel is optional. When present, stdout or stderr is parsed as
one JSON value (`format: "json"`) or as a JSON-lines sequence
(`format: "jsonl"`). `ignore_keys` removes measurement-only fields recursively
before canonicalization. The optional `expected` value is checked after the
same projection. When no semantic channel is declared, the stdout hash is the
semantic identity. In either case, the first measured baseline result becomes
the comparison reference; every later baseline and candidate result must match
it. A workload may set `compare_stderr` when stderr is part of its stable
observable contract.

A workload can also declare `inner_metrics`. Each entry names a numeric field
`path`, and may give a JSON object `key` list to identify JSON-lines records
and a `unit`. For JSON-lines, rows that lack any declared key field are
metadata and are skipped by that metric; selected rows must contain the path.
The runner extracts these values before applying `ignore_keys`, then reports
them in separate sample, pair, and `inner` summary fields. A missing field on a
selected row, nonnumeric value, or duplicate key fails the run. The runner's
process wall/user/system/RSS metrics include process startup and all work done
by the executable, while an inner metric is the executable's own declared
measurement and is never substituted for process timing.

For example:

```json
{
  "schema": 1,
  "lane": "release-unobserved",
  "profile": "release",
  "features": [],
  "instrumented": false,
  "toolchain": { "rustc": "...", "cargo": "...", "target": "..." },
  "environment": { "LC_ALL": "C.UTF-8" },
  "workloads": [
    {
      "name": "command-core-matrix",
      "args": [
        "--case",
        "plain",
        "--iterations",
        "100000",
        "--warmups",
        "64",
        "--observer",
        "disabled"
      ],
      "files": [],
      "semantic": {
        "stream": "stdout",
        "format": "jsonl",
        "ignore_keys": ["elapsed_ns", "ns_per_iteration"]
      },
      "inner_metrics": [
        {
          "name": "elapsed_ns",
          "path": ["elapsed_ns"],
          "key": ["case", "storage", "delivery", "observer"],
          "unit": "ns"
        },
        {
          "name": "ns_per_iteration",
          "path": ["ns_per_iteration"],
          "key": ["case", "storage", "delivery", "observer"],
          "unit": "ns/iteration"
        }
      ],
      "outputs": [],
      "expected_exit": 0
    }
  ]
}
```

For a single-result command-core executable, stdout should contain one UTF-8
JSON object and diagnostics should go to stderr. The current command-core
matrix emits one metadata JSON line followed by JSON-lines result records, so a
matrix workload should use `format: "jsonl"` and ignore its
`elapsed_ns` and `ns_per_iteration` fields. Stable semantic fields belong
outside the ignored timing fields. A useful single-result shape is:

```json
{
  "schema": 1,
  "semantic": { "status": "ok", "work_units": 4096, "checksum": "..." },
  "elapsed_ns": 123456,
  "user_cpu_ns": 120000,
  "system_cpu_ns": 0,
  "max_rss_kib": 2048
}
```

The runner does not require these field names; the manifest decides which
fields are measurement-only. This keeps the executable's semantic interface
independent of the runner's timing fields while allowing baseline-core output
to be validated without scraping human-readable text. For the matrix, the
semantic projection checks every JSON-lines record, including the stable
operation counters and checksums, while excluding only the timing fields.

## Sampling and receipt

For each workload, `--pairs N` creates `N` measured baseline/candidate pairs.
Even-numbered pairs run baseline then candidate (`AB`); odd-numbered pairs run
candidate then baseline (`BA`). Each side receives `--warmups` discarded
processes immediately before its measured process. Every process gets a unique
sample directory, a fresh process group, and the same workload arguments and
environment. Sample directories are retained under `--run-root` for audit.

The timer starts immediately before process creation and stops after process
reaping. It therefore excludes runner setup, Git checks, hashing, directory
creation, and receipt serialization. Process startup and the selected stdout
capture remain part of wall time. `/usr/bin/time` supplies user CPU,
system CPU, and maximum RSS when available; otherwise user and system CPU use
the child resource counter and RSS is recorded as unavailable. A timeout kills
the complete process group and records a failed sample.

The output is JSONL with one metadata record, raw warm-up and measured sample
records, one pair record per pair, and a final summary record. Every record
contains the schema and evidence lane. The metadata contains the profile,
features, manifest and workload hashes, both Git and binary identities,
host/affinity/load observations, environment overrides, warm-up and pair
counts, timeout, CPU selection, and resource-metric source. Sample records
contain the exact executable argv, timed argv, sample directory, wall/user/
system/RSS values, exit and timeout status, stdout/stderr byte counts and
hashes, semantic hash, declared output identities, and validation errors.
The summary reports integer min/median/mean/max values for each side and paired
candidate-minus-baseline deltas and ratios. A validation failure writes a
`FAIL` summary and exits nonzero; it never produces a passing partial result.

## Invocation

Build both binaries before invoking the runner. The following is a short local
smoke shape; production runs should use the exact pinned workload manifest and
more than one pair:

```bash
python3 scripts/paired-measure.py \
  --manifest /absolute/path/workloads.json \
  --lane release-unobserved \
  --baseline-worktree /absolute/path/baseline-worktree \
  --baseline-revision 0123456789012345678901234567890123456789 \
  --baseline-binary /absolute/path/baseline-worktree/target/release/umber \
  --candidate-worktree /absolute/path/candidate-worktree \
  --candidate-revision abcdefabcdefabcdefabcdefabcdefabcdefabcd \
  --candidate-binary /absolute/path/candidate-worktree/target/release/umber \
  --pairs 5 \
  --warmups 1 \
  --output /absolute/path/target/paired/receipt.jsonl
```

The runner accepts `--cpu-list` for a child affinity such as `4`, and
`--timeout-seconds` for a per-process guard. It has no `--build` or
`--cargo` option by design. A receipt from a profiling-enabled executable must
name `profiling-structural`; there is no automatic promotion of an
instrumented run to production evidence.
