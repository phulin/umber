# umber2-awgc.5.3.8: Fixed Job-Clock Authority

## Finding

The fused unexpandable and expansion paths did not cause the reported
secondary-work drift. The historical performance command omitted
`SOURCE_DATE_EPOCH`, so it did not fully identify the TeX workload. TeX82
§241 initializes `\time`, `\day`, `\month`, and `\year` from the host before
input begins. The loaded LaTeX format's l3kernel `everyjob` hook immediately
expands those parameters into job constants.

The original 6M command receipt was created at 2026-08-18 19:13:54 UTC.
Pinning that instant as `SOURCE_DATE_EPOCH=1787080434` on the unchanged
pre-fusion commit `06dd08bc5` exactly restores the historical vectors. Leaving
the clock live on 2026-08-19 produces the alleged fused-path delta on that same
pre-fusion code.

## Earliest divergence

The exact command, ordered 105-key prefetch closure, restored cache, source,
format, distribution, locale, and binary were held fixed. The only changed
input was the job clock.

At fuel 545, l3kernel's job-constant setup consumes TeX's value rendering of
`\time`: the authority run sees the four tokens `1153`, while a same-day
midnight negative control sees the one token `0`. At fuel 546, the authority
is still matching rendered digits while the negative control has reached the
next control sequence. That is the first secondary-work divergence. Temporary
bounded probes were removed before the clean rebuild.

This is canonical semantic input, not counter noise. Adjusting counters,
charging synthetic work, or reverting either fused architecture would compare
different TeX jobs and conceal the missing authority field.

## Corrected fixed-prefix authority

The clean one-job profiling build of pre-fusion commit `06dd08bc5` produced
binary SHA-256
`3183d31d983920c1ec493f4e1fb1d56a7f81363ad062fae0b1abc0caf6111dad`.
With `SOURCE_DATE_EPOCH=1787080434` and `LC_ALL=C.UTF-8`, the exact restored
rows are:

| Boundary | Fuel charges | Token-frame steps | Expanded deliveries | Meaning lookups | Scanner tokens | Write expansions |
| -------- | -----------: | ----------------: | ------------------: | --------------: | -------------: | ---------------: |
| 6M       |    6,000,000 |         5,999,815 |             507,410 |       1,718,333 |      5,352,087 |              588 |
| 12M      |   12,000,000 |        11,999,815 |           1,177,349 |       3,506,292 |     10,599,869 |            1,182 |

The 6M and 12M stderr receipts have SHA-256
`bca5e797c7821487221c27e46102284505f1f21e110593e0469b3ccffa4d308f`
and
`ee808ac0e9e0974ef08ccf0c1bd8f2a7a60609a69f46e0313db267847caf8770`.
Both returned typed status 1 at exact fuel exhaustion and emitted empty
stdout.

The machine-readable authority is
[`umber2-awgc.5.3.8-authority.json`](umber2-awgc.5.3.8-authority.json). Later
performance rows must record the pinned clock and locale alongside the
content-addressed assets, command, prefetch ordering, cache, guards, and fuel.
