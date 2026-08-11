# umber2-sdpz.49.3.2: Private Retry Checkpoints

## Reproduction and ownership

Row 2606.03990 was reproduced from source SHA-256
`164d46b605e345a852dfe144d2536a78884694f77f7e379e69dcacde86bcfde5`,
schema-11 pdfLaTeX format
`0cfb18d9b9f4548fab57f3e003b1d9c886a9b3bbc633781e67430ae2f0669242`,
and explicit distribution manifest
`43a31da364e4607957a38da10dabff227657d607d1845d502204adfd5d002e4b`.
The cache, format, distribution, source, materialization, and output remained
under `target/umber2-sdpz.49.3.2`. The unchanged offline guards were 100,000,000
canonical actions, 120 seconds, and 1,536 MiB. The pre-edit candidate reached
status 124; its engine log has SHA-256
`3669a48204ac70d0df7b47857923b0e4dc69c61fa7defb935558b1bc934d8ef4`.

The exhaustive differential tracer was clean with zero gating and advisory
divergences. Story completed one page/680 DVI bytes and Gentle completed 97
pages/263,424 DVI bytes. A fresh 30-second profile captured 10,821 samples with
zero lost and led through command delivery plus private checkpoint/state-hash
projection. `umber2-sdpz.60` owns TeX82 §394's macro-argument brace matcher;
that frame was not a leading owner, so the state/executor retry boundary was
disjoint before editing.

## Canonical invariant and fix

TeX82 §§1030--1038 fetch and dispatch commands inside `main_control`; they do
not create a semantic checkpoint after every command. The repository's
stepwise contract likewise requires private step savepoints to be unhashed and
the unobserved production step to contain at most 256 complete operations.

`Universe` now exposes an opaque rollback-only `LocalRetrySnapshot`. It retains
the aggregate store, world, input, page, PDF, dependency, and geometry roots
needed for same-step replay without constructing or advancing a durable state
hash. The unobserved `CanonicalStepRunner` holds one such retry point across a
bounded 256-operation production chunk and stops at a named boundary,
group-lineage change, terminal result, observation boundary, or world effect.
Returning on an effect lets the host publish same-run output and enforce its
pending-effect budget before later input continues. Public diagnostic and observed-delivery paths remain
single-operation. Focused controls prove that private capture does not advance
the snapshot serial or checkpoint hash, exact retry restores state,
provenance, and geometry, ordinary commands share one retry point, and a later
resource need rolls the entire bounded prefix back before exact replay. Effect
controls prove both immediate budget enforcement and same-run output overriding
an earlier authoritative-negative probe.

Exact two-phase compatibility TRIP and the official e-TRIP artifact gate are
unchanged and green. The exhaustive tracer remains clean.

## Observed successor

The final candidate binary SHA-256 was
`9cf60cdcefdf747ee93ce117ed51381f656628b147c74c4d2605c23aa86b4154`.
The exact row still reached status 124; its final engine log SHA-256 is
`7f6f6648126371f97861656fd364faf4c817fe782e28081d55307df8c83468c9`.
This is a later performance owner rather than the repaired private checkpoint:
a delayed 70--110 second profile captured 18,387 samples with zero lost and
reported `main_memory_usage_inner` at 28.47% self, `MacroStore::get` at 8.87%,
`TokenStore::get` at 6.28%, and the environment root walk at 4.45%; command
delivery had fallen to 1.98% and private checkpoint projection was absent.
The profile has SHA-256
`e22623915b86463ab0ed874b8e78cf41779608bcecf88fd8fbff761519d58526`.
That observed allocator-projection boundary is filed as `umber2-sdpz.72`.
