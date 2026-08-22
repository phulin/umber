use serde_json::Value;
use sha2::{Digest, Sha256};
use test_support::pdf_fixture::{Dictionary, ValidPdfFixture, array, name, reference};

use super::*;

const ACTIVE_TEST: &str = "crates/umber/src/pdftex/tests/retained_fixture_properties.rs::retained_pdftex_extension_fixtures_compare_oracle_projections";

struct ActualChannels {
    status: String,
    terminal: String,
    log: String,
}

#[test]
fn retained_pdftex_extension_fixtures_compare_oracle_projections() {
    let catalogue: Value = serde_json::from_str(include_str!(
        "../../../../../tests/pdftex-properties/catalogue.json"
    ))
    .expect("parse pdfTeX extension property catalogue");
    let properties = catalogue["properties"]
        .as_array()
        .expect("catalogue properties");
    let owned = properties
        .iter()
        .filter(|property| property["active_test"] == ACTIVE_TEST)
        .collect::<Vec<_>>();
    assert_eq!(owned.len(), 8, "runner-owned fixture census changed");

    for property in owned {
        let id = property["id"].as_str().expect("property id");
        let case = property["case"].as_str().expect("property case");
        let expected_success = property["expected_success"]
            .as_bool()
            .expect("expected success");
        let reference = String::from_utf8(
            test_support::read_repository_asset(format!(
                "tests/pdftex-properties/fixtures/{case}/expected.ref"
            ))
            .unwrap_or_else(|error| panic!("read {case} reference: {error:#}")),
        )
        .unwrap_or_else(|error| panic!("{case} reference is not UTF-8: {error}"));
        let (reference_success, terminal, log) = reference_channels(&reference);
        assert_eq!(
            reference_success, expected_success,
            "{id} status projection drifted"
        );
        let actual = execute(case);

        for observation in property["observations"]
            .as_array()
            .expect("property observations")
        {
            let channel = observation["channel"].as_str().expect("channel");
            let projection = observation["projection"].as_str().expect("projection");
            let oracle = match channel {
                "status" => {
                    if reference_success {
                        "success"
                    } else {
                        "error"
                    }
                }
                "terminal" => terminal,
                "log" => log,
                other => panic!("{id} invalid channel {other}"),
            };
            if channel != "status" {
                assert!(
                    normalize(oracle).contains(&normalize(projection)),
                    "{id} {channel} oracle lacks {projection:?}"
                );
            }

            let observed = match channel {
                "status" => actual.status.as_str(),
                "terminal" => actual.terminal.as_str(),
                "log" => actual.log.as_str(),
                _ => unreachable!(),
            };
            let matches = normalize(observed).contains(&normalize(projection));
            match observation["disposition"].as_str().expect("disposition") {
                "pass" => assert!(
                    matches,
                    "{id} {channel} lacks passing projection {projection:?}: {observed}"
                ),
                "xfail" => {
                    assert!(
                        !matches,
                        "{id} {channel} unexpectedly matches {projection:?}; close {} and promote this observation to pass",
                        observation["bug"].as_str().expect("xfail bug")
                    );
                    assert!(
                        xfail_matches_exact_actual(observation, channel, observed),
                        "{id} {channel} xfail changed to an unrelated divergence: {observed}"
                    );
                }
                other => panic!("{id} invalid disposition {other}"),
            }
        }
    }
}

fn xfail_matches_exact_actual(observation: &Value, channel: &str, observed: &str) -> bool {
    if channel == "status" {
        return observation["actual"].as_str() == Some(observed);
    }
    let digest = format!("{:x}", Sha256::digest(normalize(observed)));
    observation["actual_normalized_sha256"].as_str() == Some(digest.as_str())
}

#[test]
fn strict_xfail_fingerprints_reject_blank_unrelated_and_different_failures() {
    let status = serde_json::json!({"actual": "success"});
    assert!(xfail_matches_exact_actual(&status, "status", "success"));
    assert!(!xfail_matches_exact_actual(&status, "status", ""));
    assert!(!xfail_matches_exact_actual(
        &status,
        "status",
        "error:other"
    ));

    let log = serde_json::json!({
        "actual_normalized_sha256": "d0bca111f8628137adc4c16f123496dcdd1d590d06cb5d9acd68b39fe656fb97"
    });
    assert!(xfail_matches_exact_actual(&log, "log", " [0]"));
    assert!(!xfail_matches_exact_actual(&log, "log", ""));
    assert!(!xfail_matches_exact_actual(
        &log,
        "log",
        "unrelated failure"
    ));
    assert!(!xfail_matches_exact_actual(&log, "log", " [1]"));
}

fn execute(case: &str) -> ActualChannels {
    let source = test_support::read_repository_asset(format!(
        "tests/pdftex-properties/fixtures/{case}/{case}.tex"
    ))
    .unwrap_or_else(|error| panic!("read {case} source: {error:#}"));
    with_pdftex_oracle_stores(|stores| {
        if case == "pdf_ximage_enquiries" {
            seed_ximage_inputs(stores);
        }
        prepare_pdftex_run_stores(stores);
        let result = run_pdf_memory_result(
            std::str::from_utf8(&source).expect("fixture source is UTF-8"),
            stores,
        );
        let returned = result
            .as_ref()
            .map(|result| result.terminal_text.as_str())
            .unwrap_or_default();
        let terminal = format!(
            "{}{}",
            String::from_utf8_lossy(stores.world().memory_terminal_output().unwrap_or_default()),
            returned
        );
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(stores.world().memory_log_output().unwrap_or_default()),
            returned
        );
        let status = match result {
            Ok(result) if result.status.is_success() => "success".to_owned(),
            Ok(_) => "error".to_owned(),
            Err(error) => format!("error:{error}"),
        };
        ActualChannels {
            status,
            terminal,
            log,
        }
    })
}

fn reference_channels(reference: &str) -> (bool, &str, &str) {
    let success = reference
        .strip_prefix("success: ")
        .and_then(|rest| rest.lines().next())
        .expect("reference success status");
    let (_, channels) = reference
        .split_once("\nstdout:\n")
        .expect("reference stdout channel");
    let (terminal, log) = channels
        .split_once("log:\n")
        .expect("reference log channel");
    (success == "true", terminal, log)
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .collect()
}

fn seed_ximage_inputs<G>(stores: &mut Universe<G>) {
    let mut pdf = ValidPdfFixture::new("1.7").expect("create three-page PDF");
    pdf.add_dictionary(
        1,
        Dictionary::new()
            .entry("Type", name("Catalog"))
            .entry("Pages", reference(2)),
    )
    .expect("catalog");
    pdf.add_dictionary(
        2,
        Dictionary::new()
            .entry("Type", name("Pages"))
            .entry("Kids", array([reference(3), reference(4), reference(5)]))
            .entry("Count", b"3"),
    )
    .expect("page tree");
    for object in [3, 4, 5] {
        pdf.add_dictionary(
            object,
            Dictionary::new()
                .entry("Type", name("Page"))
                .entry("Parent", reference(2))
                .entry("MediaBox", b"[0 0 10 20]"),
        )
        .expect("page");
    }
    pdf.set_trailer_entry("Root", reference(1))
        .expect("trailer root");

    // Keep these byte-for-byte equivalent to the typed resources used to
    // capture the retained oracle. In particular, the PNG is grayscale-alpha
    // (IHDR color type 4), not opaque grayscale: pdftex.web §1552's
    // `read_image` reserves its transparency companion after image object 1,
    // so the following JPEG and PDF image objects are 3 and 4.
    let png = vec![
        0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44,
        0x52, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5,
        0x1c, 0x0c, 0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64,
        0xf8, 0x0f, 0x00, 0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00,
        0x49, 0x45, 0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
    ];
    let jpeg = vec![
        0xff, 0xd8, 0xff, 0xc0, 0x00, 0x11, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x03, 0x01, 0x11, 0x00,
        0x02, 0x11, 0x00, 0x03, 0x11, 0x00, 0xff, 0xd9,
    ];
    for (path, bytes) in [
        ("depth8.png", png),
        ("depth12.jpg", jpeg),
        (
            "three-pages.pdf",
            pdf.finish().expect("serialize three-page PDF"),
        ),
    ] {
        stores
            .world_mut()
            .set_memory_file(path, bytes)
            .unwrap_or_else(|error| panic!("seed {path}: {error}"));
    }
}
