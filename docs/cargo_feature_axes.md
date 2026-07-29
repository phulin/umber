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
crates, is the defect this contract exists to prevent. `instrumentation` and
`trip-instrumentation` were one axis under two names, and `profiling-stats`
named an axis in a way that read like a pair with `profiling-runner`, which
is not an axis at all (see §3).

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

`parity-harness` previously spelled this `trip-instrumentation`. It still
gates `dep:tex-command`, which is the legitimate use of a feature described
in §3; only the name changed, to the one every other crate uses.

### `profiling` -- owner `tex-state`

Enables the compile-time counters dedicated profiling builds read. These are
ephemeral process-wide counters that never participate in engine state. This
was `profiling-stats`. The similar-looking `profiling-runner` is not part of
this axis and is not an axis: it is `required-features` on the
`gentle-profile` binary (see §3).

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

## 3. `required-features` is the fifth thing, and it stays

Three features gate an artifact rather than an axis: `tex-out/dvi-tools`,
`umber/profiling-runner`, and `parity-harness/reference-tools` are each
`required-features` on a `[[bin]]`. They look like clutter next to the four
axes, and moving each binary into its own `tools/` crate would delete all
three names.

Measured against build time, that trade is a loss, and it was tried and
reverted rather than reasoned about abstractly. `required-features` already
costs the routine gates exactly nothing: `cargo test --tests --workspace` and
`cargo clippy --workspace --all-targets` both skip a `[[bin]]` whose features
are unmet, so `gentle-profile`'s 2,433 lines and its `umber` dependency are
already absent from every routine build. A `tools/` crate is a workspace
member, so both gates would start building it -- and keeping the old behavior
would then need a `Deferral` in `scripts/run-native-tests.py`, two `--exclude`
flags in each clippy pass, and a new members-linted-elsewhere escape hatch in
`check_coverage`, which today requires every member to be linted by some pass.
Three declarations and a new mechanism, to buy back the build time the feature
was already saving.

So the rule is not "crates beat features". It is:

- an optional **dependency** is a feature (`reference-tools` gates
  `dep:refexec`);
- an optional **artifact** is `required-features` on its target;
- everything else is one of the four axes, or it does not exist.

`reference-tools` has a second reason to stay: the code behind it is library
code sharing private helpers (`staged_source_dir`, `write_triage_bundle`,
`read_manifest`, `copy_source`, `first_diff`, and the
`TriageInput`/`TraceBundle`/`EngineDvi` types) with the ungated functions
`umber`'s tests call. Splitting it would make roughly a dozen private items
`pub` for no reason but to enable the split, which
[Rust Testing Policy](testing_policy.md) §9 exists to prevent.

## 4. No non-empty `default`

Every engine crate declares either no `default` or an empty one. The
resolution a released `umber` builds in must be the resolution you get by
typing nothing: a non-empty `default` makes `--no-default-features` one more
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
