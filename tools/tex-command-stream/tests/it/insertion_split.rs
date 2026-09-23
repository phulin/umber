//! Oracle-backed regression for page-region succession after a split insertion.

#![allow(
    clippy::disallowed_methods,
    reason = "this host-only fixture test reads its committed oracle channels"
)]

use std::fs;

use tex_command_stream::semantic::channels::{CapturedChannels, StreamChannel, normalize_channel};
use tex_command_stream::semantic::{execute_with_provider, load_suite, project};
use umber::{FormatCacheStore, FormatWorkerLauncher, PreparedFormatProvider};

#[test]
fn split_insertion_survives_page_successor_and_matches_oracle_streams() {
    let declared = load_suite()
        .expect("valid command-semantic corpus")
        .into_iter()
        .find(|declared| {
            declared.domain == "page-output" && declared.case.id == "insertion-split-footnote"
        })
        .expect("split-insertion oracle fixture");
    let root = tempfile::TempDir::new().expect("hermetic format cache");
    let provider = PreparedFormatProvider::with_store(
        FormatCacheStore::new(root.path()),
        FormatWorkerLauncher::registered_libtest("umber_format_worker_bootstrap"),
    );
    let source =
        fs::read(declared.fixture_dir.join(&declared.case.source)).expect("fixture source");
    let run = execute_with_provider(&source, &declared.case, &provider)
        .expect("split insertion must finish both page shipouts");
    let actual = CapturedChannels::capture(&run);
    assert_eq!(actual.status, "clean");

    // TeX82 §§1018, 1020--1021 carry the split remainder into the next page.
    // The projection's page-plan hashes and total event count are independent
    // compatibility claims; this regression checks the reference streams and
    // the two observable shipout boundaries that the panic used to prevent.
    assert_eq!(
        &project(&run, &declared.case.projection)[..2],
        &declared.case.expected[..2]
    );
    for channel in [
        StreamChannel::Terminal,
        StreamChannel::Log,
        StreamChannel::Dvi,
    ] {
        let expected = fs::read(
            declared
                .fixture_dir
                .join(format!("expected.{}", channel.name())),
        )
        .expect("committed oracle channel");
        // DVI normalization replaces only the preamble's k-byte banner;
        // every remaining byte, including lengths and pointers, must agree.
        assert_eq!(
            normalize_channel(channel, actual.stream(channel)).expect("valid actual channel"),
            normalize_channel(channel, &expected).expect("valid oracle channel"),
            "{} differs from the committed TeX82 oracle",
            channel.name()
        );
    }
    assert!(actual.stream(StreamChannel::Effects).is_empty());
    assert!(actual.stream(StreamChannel::Diagnostics).is_empty());
}
