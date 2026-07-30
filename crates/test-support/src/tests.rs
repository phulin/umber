use crate::read_fixture;

#[test]
fn hello_fixture_is_committed() {
    let expected = read_fixture("hello", "hello", "log");
    assert!(
        expected.contains("hello umber"),
        "hello fixture should keep the reference message"
    );
}

#[test]
#[allow(clippy::disallowed_methods)] // Host-only audit of committed oracle sources.
fn every_oracle_close_effect_omits_stale_file_name_globals() {
    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for dialect in ["tex82", "etex26", "pdftex14027"] {
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
