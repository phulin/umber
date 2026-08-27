# `umber2-66p0.30`: canonical command resolution ownership

## Evidence boundary

The comparison uses the authenticated arXiv `2606.12566` workload from the
post-copy-wave profile: selected `ArXiv.tex` SHA-256
`816440f61d611fa57cef802e6f372b9337beef1cc4e48e5536d4bad1014ec537`,
schema-12 format object `ahash64-v1-2b924b5bba05d8a0` with SHA-256
`ddd082db722654fd9c47e1080d8e128d1a5b0cfb3186f9e3bc01f8943f559ad4`,
packed distribution root `721e833071d92bba`, and the 123-key closure with
SHA-256
`e4f4113c9057af88c239d40d3041f598871a0a7a895f8bd63f89d7c77682ab7e`.

The post-wave base executable has SHA-256
`5b39fc8c1eb2c724ad94b0c0dd4d1aaca21dc20beb7888079441f9f3d5cf6f20`
and build ID `7427532695e82663182e292eb2209ce9bdf64aab`. One candidate
force-frame-pointer ELF served both final candidate rows. It has SHA-256
`51634aba50626ec0472b70916108d8c026c2cd455e76fcd8b0e85fcf26d09218`
and build ID `8363ab05a6f1a589f9a64949b99795b625070728`.

One outer `flock /tmp/umber-perf-host.lock` covered the final before/after
control and `cycles:u` rows. CPU `some` and `full` pressure both had
`avg10=0.00` at the start and end, and the saved process censuses contained no
Cargo, rustc, Umber, or perf peer. Issue-private receipts remain under
`target/umber2-66p0.30/`.

## Concrete attribution

Canonical delivery previously unpacked `TracedTokenWord` solely to classify
control-sequence and active-character work, then resolution unpacked the same
word again to obtain the meaning. Resolution now owns the operational lookup
count at the exact match that performs the lookup. No classifier is returned
to the caller or retained in the command.

Every resolution also classified and stored a `CommandIdentity`, although the
ordinary identity is consumed only when observation publishes the command.
Observation now derives ordinary identity from the already-resolved meaning.
The mutually exclusive exceptional states for `\noexpand`'s frozen relax and
outer-validity's recovery space share one `EffectiveCommandAdjustment` instead
of a retained identity plus a recovery boolean.

Finally, `macro_observation_operand` was removed. Repository-wide source
inspection showed that the field was initialized and reset only to `None`;
observation already derives its operand from effective identity and meaning.
This removes an empty eight-byte field from delivery, clone, backup,
suspension, equality, and hashing without introducing a cache, special path,
heap owner, or lifetime mechanism.

The hot resolver text shrank from `0x74f` to `0x507` bytes. A rejected
intermediate design returned a lookup-classification boolean to the caller;
paired evidence showed that handoff enlarged caller work, so it is not part of
the production change.

## Exact endpoint and cycles

Every accepted row reproduced
`(20000000,19913119,2218327,6020965,16785710,4011)` for fuel charges,
token-frame steps, expanded deliveries, meaning lookups, scanner tokens, and
write expansions. Distribution telemetry was also identical.

The 199 Hz frame-pointer captures contain 1,508 and 1,578 samples with zero
lost samples. Rows are non-additive: self cycles name the leaf bucket, while
inclusive cycles name every sample with the function in its full ancestry.

| Owner                                    |   Base cycles | Candidate cycles | Absolute change | Relative change |
| ---------------------------------------- | ------------: | ---------------: | --------------: | --------------: |
| `CurrentCommand::resolve_into` self      | 1,360,648,420 |      991,557,312 |    -369,091,108 |         -27.13% |
| `CurrentCommand::resolve_into` inclusive | 1,530,693,075 |    1,136,349,106 |    -394,343,969 |         -25.76% |
| `get_next_canonical` self                | 1,792,697,591 |    1,973,476,217 |    +180,778,626 |         +10.08% |
| `get_next_canonical` inclusive           | 5,370,805,375 |    5,113,616,196 |    -257,189,179 |          -4.79% |
| shared `memmove` self                    |   966,881,397 |      798,836,397 |    -168,045,000 |         -17.38% |
| shared `memmove` inclusive               |   997,655,985 |      836,189,917 |    -161,466,068 |         -16.18% |

Total approximate weighted cycles moved from 17,853,501,660 to
18,586,866,348, an increase of 733,364,688 cycles (4.11%). The result is
therefore a targeted resolver and canonical-delivery ancestry improvement,
not a claim of whole-engine improvement from this single capture. Guarded
control wall time moved from 8.04 to 8.12 seconds and user time from 8.70 to
8.88 seconds.

## Verification

The focused `tex-command` suite passes 242 unit and 18 integration tests. The
complete `cargo test -q --tests` routine suite passes. The warmed
`destination_directed_warm_delivery` cutover row performs 24,576 deliveries
with zero allocation calls and zero requested bytes. `scripts/check.sh`
passes dprint, Biome, rustfmt, and clippy.
