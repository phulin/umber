use crate::{normalize, read_fixture};

#[test]
fn diagnostic_normalization_retains_box_reports() {
    let report = "Overfull \\hbox (5.2778pt too wide) in paragraph at lines 4--4\n\
                  []\\tenrm x\n\n\
                  \\hbox(4.30554+0.0)x0.0 []\n";

    assert_eq!(normalize::exec_log(report), report);
    assert_eq!(normalize::box_dump(report), report);
}

#[test]
fn diagnostic_normalization_retains_every_context_kind_and_continuation() {
    let context_headers = [
        "<argument>",
        "<template>",
        "<to be read again>",
        "<recently read>",
        "<inserted text>",
        "<output>",
        "<everypar>",
        "<everymath>",
        "<everydisplay>",
        "<everyhbox>",
        "<everyvbox>",
        "<everyjob>",
        "<everycr>",
        "<mark>",
        "<write>",
        "\\macro #1->",
        "l.42",
        "<*>",
        "<read 1>",
    ];

    for header in context_headers {
        let report = format!("{header} before\n                  after\n");
        assert_eq!(normalize::exec_log(&report), report, "context {header}");
        assert_eq!(normalize::box_dump(&report), report, "context {header}");
    }

    let report = "PDF statistics:\n 1 volatile statistic\nretained diagnostic\n";
    assert_eq!(normalize::exec_log(report), "retained diagnostic\n");
}

#[test]
fn hello_fixture_is_committed() {
    let expected = read_fixture("hello", "hello", "log");
    assert!(
        expected.contains("hello umber"),
        "hello fixture should keep the reference message"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-only cross-checkout process regression.
fn fixture_root_follows_the_runtime_checkout() {
    const CHILD_EXPECTATION: &str = "TEST_SUPPORT_RUNTIME_ROOT_EXPECTATION";
    if let Some(expected) = std::env::var_os(CHILD_EXPECTATION) {
        assert_eq!(
            read_fixture("runtime-root", "checkout", "txt"),
            expected.to_string_lossy()
        );
        return;
    }

    let runtime_checkout = tempfile::tempdir().expect("create runtime checkout");
    let fixture = runtime_checkout
        .path()
        .join("tests/corpus/runtime-root/checkout.expected.txt");
    std::fs::create_dir_all(fixture.parent().expect("fixture has parent"))
        .expect("create runtime fixture directory");
    std::fs::write(&fixture, "selected at runtime").expect("write runtime fixture");
    let initialized = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(runtime_checkout.path())
        .status()
        .expect("initialize runtime checkout");
    assert!(initialized.success(), "initialize runtime checkout");

    let status = std::process::Command::new(std::env::current_exe().expect("locate test binary"))
        .args([
            "--exact",
            "tests::fixture_root_follows_the_runtime_checkout",
        ])
        .env(CHILD_EXPECTATION, "selected at runtime")
        .current_dir(runtime_checkout.path())
        .status()
        .expect("re-execute test binary from runtime checkout");
    assert!(
        status.success(),
        "reused test binary must read the runtime checkout"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-only audit of committed oracle sources.
fn every_oracle_close_effect_omits_stale_file_name_globals() {
    let repository = crate::repository_root();
    for dialect in ["tex82", "etex26", "pdftex14029"] {
        let change = std::fs::read_to_string(
            repository
                .join("tests")
                .join(format!("{dialect}-oracle"))
                .join("instrumentation.ch"),
        )
        .unwrap_or_else(|error| panic!("read {dialect} oracle instrumentation: {error}"));
        assert!(
            change.contains("umber_trace_effect(3,j,4,0);"),
            "{dialect} close instrumentation must publish no value (TeX82 §§1374, 1378)"
        );
        assert!(
            !change.contains("umber_trace_effect(3,j,3,0);"),
            "{dialect} close instrumentation must not publish stale cur_name globals"
        );
    }
}
