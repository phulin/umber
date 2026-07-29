# Cargo Feature Axes

Status: repository contract for what a Cargo feature may mean in this workspace

Scope: which features the workspace declares, what each one is allowed to
change, and which crate owns each declaration. For how the lint gate proves it
covers each resolution, see
[Testing Infrastructure](testing_infrastructure.md#what-the-clippy-gate-covers).

---

## 1. The rule

A Cargo feature in this workspace may only do one of four things, and the
thing it does is named by which of four axes it belongs to. Every feature is
declared by exactly one owning crate. Every other crate that needs it declares
a pass-through with the same name and nothing else:

```toml
observe = ["tex-command/observe"]
```

A feature that does two things at once, or that is spelled differently in two
crates, is the defect this contract exists to prevent. Eight names covering
four purposes is what the workspace had before this contract:
`instrumentation` and `trip-instrumentation` were one axis under two names,
and `dvi-tools`, `reference-tools`, and `profiling-runner` were three names
for `required-features` on a `[[bin]]`.

Two axes are deliberately _not_ merged. `shadow` and `testing` both mean
"this build may check itself in ways a shipped engine must not", but they
cost different things: `testing` is a compile-time API widening that every
crate's unit tests already enable through dev-dependencies at no runtime
cost, while `shadow` mirrors every environment write into a parallel map and
gates additional replay tests. Folding them into one name would switch that
mirror on for the whole routine test tier and change which tests run there.
Same sentence, different price; two axes.

## 2. The four axes

### `observe` -- owner `tex-command`

Enables the canonical semantic observation vocabulary: the
`CommandObservation` record types, their spelling resolution, and the
`CommandObserver` trait. It never changes delivery, expansion, scanning,
conditional, or alignment semantics, and an engine built with it enabled must
produce byte-identical artifacts to one built without it.

Pass-throughs: `tex-exec`, `umber`, `parity-harness`, `tex-command-stream`.

`parity-harness` previously spelled this `trip-instrumentation` and made
`tex-command` an optional dependency to express it. The dependency is no
longer optional; the feature name is the same one every other crate uses.

### `profiling` -- owner `tex-state`

Enables the compile-time counters dedicated profiling builds read. These are
ephemeral process-wide counters that never participate in engine state. This
was `profiling-stats`; the `profiling-runner` half of the old pair was the
`gentle-profile` binary, which is now its own crate (see below).

Pass-throughs: `tex-lex`, `tex-expand`, `tex-exec`, `umber`.

The `umber run --profiling-stats` CLI flag keeps its name. It is a command
line surface, not a feature, and renaming it would break invocations in
`scripts/measure-allocation-owners.sh` and `scripts/measure-node-arena.sh`
for no gain.

### `shadow` -- owner `tex-state`

Mirrors every environment write into an independent map so a verification run
can compare the two, and enables the replay tests that consume the mirror.
This is production-like -- it exposes no handle constructors -- but it is not
free, so no routine gate enables it.

Pass-throughs: `tex-expand`, `umber`.

### `testing` -- owner `tex-state`

Widens `tex-state`'s API with the raw storage-word escape hatches replay and
fuzz coverage need. Engine production builds must not enable it. Unlike
`shadow` it costs nothing at runtime, which is why every crate's unit tests
enable it through a dev-dependency and the routine tier is unaffected.

No pass-throughs: consumers depend on `tex-state` with the feature directly
in `[dev-dependencies]`.

`shadow` and `testing` are the two axes Cargo cannot enforce, because
features are additive by contract and "must not be enabled" is not additive.
`scripts/check-lint-passes.py` carries that constraint instead: its
`shipping` pass declares the exact feature set it expects Cargo to resolve,
and a manifest edit that pulls either into that resolution fails the gate.

## 3. Opt-in binaries are crates, not features

`required-features` on a `[[bin]]` is not one of the three axes, and the
workspace no longer uses it. An artifact that should stay out of ordinary
builds gets its own crate under `tools/`, because a crate is the unit Cargo
already has for "build this only when asked". Three features existed solely
to hold three binaries out of the default build:

| was                        | is now                     |
| -------------------------- | -------------------------- |
| `tex-out/dvi-tools`        | `tools/texout-dvitype`     |
| `parity-harness/reference-tools` | `tools/parity-harness-cli` |
| `umber/profiling-runner`   | `tools/gentle-profile`     |

Each is excluded from the workspace's `default-members` and run by
`scripts/check-tools.sh`, exactly as its feature-gated predecessor was.

## 4. No non-empty `default`

Every engine crate declares either no `default` or an empty one. The
resolution a released `umber` builds in must be the resolution you get by
typing nothing: a non-empty `default` makes `--no-default-features` a fourth
resolution that no lint pass covers and no test exercises.

## 5. Adding a feature

Don't, unless it belongs to one of the four axes above. If it does:

1. add it to the owning crate, or a pass-through to a consuming crate;
2. add it to a pass in `scripts/check-lint-passes.py`, or to that script's
   `UNCOVERED_ENABLED_FEATURES` with a reason naming the tier that covers it.
   A new feature is uncovered by construction, so skipping this fails the
   gate rather than silently narrowing it;
3. if it is a new axis, amend this document first. A fifth axis is a design
   change, not a manifest edit.
