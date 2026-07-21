# PNG import prototype

This standalone host-only benchmark compares the former custom PNG alpha path,
`png::Reader`, and the adopted `png::StreamingDecoder` path. It is outside the
root workspace and does not execute Umber, so the Umber process watchdog does
not apply.

Run it against the three RGBA inputs used by arXiv sample row `2402.06118`:

```bash
cargo run --release --manifest-path benchmarks/png-import/Cargo.toml -- \
  third_party/arxiv-sample-100/sources/2402.06118/images/framework.png \
  third_party/arxiv-sample-100/sources/2402.06118/images/qualitative_2.png \
  third_party/arxiv-sample-100/sources/2402.06118/images/teaser.png
```

The inputs contain 16,164,821 pixels and have SHA-256 digests
`9797cadaa233ce4adb112831831dcecb3648d48589d0a57824924f636779b22d`,
`a918ab790a7e665c0f4729ab8324ab883a8d9b1e54fe061f1869bd4bc965a478`,
and `b236170db95e678ceb090eb8bf2e381c15e653f63c4563fb08df32d949f0915`.
The benchmark reads inputs before timing and reports the median of nine complete
decode/split/level-1-encode iterations. Output hashes cover the concatenated
color and alpha streams. Reader output is intentionally unfiltered, while the
other paths preserve each source filter byte.

On the July 2026 reference Apple Silicon host, custom, Reader, and streaming
medians were 157.513, 137.617, and 125.462 ms. Reader increased the two streams
from 7,179,894 to 9,857,012 bytes and used an estimated 178,256-byte row working
set. Streaming retained the custom byte count and hashes with a 111,740-byte
bounded working set, versus 47,187 bytes for the custom path.
