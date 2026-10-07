# Input rows own individual replay words

A one-token backup has one authoritative input row and one traced spelling.
Appending that spelling to the shared transient word vector creates an offset,
a second container capacity check, and a vector retirement obligation for a
value that already fits inside the row. Scanner lookahead repeats this work
for every failed keyword or numeric terminator.

The actual input storage union should own a traced word directly for a single
replay. The common frame keeps the same identity, relative position, limit of
one and semantic source context. The word owner carries its actual replay kind
and suppression/retirement flags. It does not hold a vector coordinate or a
second ownership treatment. Admission initializes that final row directly,
without touching the transient vector. Retirement drops the row; no word-vector
length changes, synchronization or later cleanup are required.

The sole native reader selects a traced slice borrowed from the word in that
row, alongside the existing packed and traced spans. Diagnostic projections
read the same spelling and origin. Full and named input captures copy that
actual scalar payload and frame. They gain no shadow input, lookahead owner,
physical cursor or compatibility route. Literal braces, source attribution,
noexpand treatment, exact fuel and every real push/retirement remain governed
by their existing consumers.

The source known to contain one word admits that owner directly. A genuine
multiword replay retains its actual transient span or immutable list. An
explicitly linkable e-TeX aftergroup replay also retains its real vector span:
it will extend and reverse that list while unsaving the group. Such lists are
identified by their actual source admission, not by converting an arbitrary
word row after publication. No one-word vector predecessor is retained for
the ordinary single-word source.

Stack conservation checks the authoritative cursor and limit first. A live row
cannot retire, so it needs no replay-kind or retirement-flag classification.
Only an exhausted row demands that treatment; native macro and argument owners
have their fixed Pop action.

Failed keyword prefix and offender backups remain separate input levels.
This ownership cut does not fuse unit probes or change expansion, logical
stack capacity, recovery chronology or reference observations. Macro argument
capture copies an individual replay spelling into its existing final argument
pool; only the real immutable macro and argument sources retain span leases.

Verify the owner with interleaved vector replays, noexpand delivery and
checkpoint replay, including origins and unchanged vector length. Existing
canonical, exact-fuel and aftergroup cases exercise the semantic boundaries.
Inspect the native CLI specialization and compare frozen scalar, macro and
original-book workloads; removing a vector operation must produce measured
work reduction rather than merely a smaller source diff.
