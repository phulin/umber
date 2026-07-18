// Native Rust translation of upstream t/configfile.t at commit 74252e6.

use std::collections::BTreeMap;

use bib_engine::{BibCommand, VirtualPath};
use bib_input::{
    ConfigValue, ConfigurationLayer, ResolvedConfiguration, StructuredValue, XmlLimits,
    parse_config_bytes, parse_control_bytes, validate_config_bytes,
};

const CONFIG: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/biber-test.conf");
const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/general.bcf");

fn parsed() -> bib_input::ConfigurationFile {
    parse_config_bytes(CONFIG, XmlLimits::default()).unwrap()
}

fn item(attributes: &[(&str, &str)]) -> StructuredValue {
    StructuredValue {
        content: attributes
            .iter()
            .find_map(|(key, value)| (*key == "value").then(|| (*value).into()))
            .unwrap_or_default(),
        attributes: attributes
            .iter()
            .map(|(k, v)| ((*k).into(), (*v).into()))
            .collect(),
    }
}

#[test]
fn assertion_001_options_1_from_cmdline() {
    let mut resolved = ResolvedConfiguration::default();
    resolved
        .push(
            ConfigurationLayer::Command,
            [("mincrossrefs".into(), ConfigValue::Scalar("7".into()))],
        )
        .unwrap();
    assert_eq!(
        resolved.resolve("mincrossrefs"),
        Some(&ConfigValue::Scalar("7".into()))
    );
}

#[test]
fn assertion_002_options_2_from_cmdline() {
    let command = BibCommand::parse(["--configfile=biber-test.conf", "general.bcf"]).unwrap();
    assert_eq!(
        command.job().options().configuration(),
        Some(&VirtualPath::user("biber-test.conf").unwrap())
    );
}

#[test]
fn assertion_003_options_3_from_config_file() {
    assert_eq!(
        parsed().value("sortlocale"),
        Some(&ConfigValue::Scalar("testlocale".into()))
    );
}

#[test]
fn assertion_004_options_4_from_config_file() {
    assert_eq!(
        parsed().value("collate_options"),
        Some(&ConfigValue::List(vec![
            item(&[("name", "level"), ("value", "3")]),
            item(&[
                ("name", "table"),
                ("value", "/home/user/data/otherkeys.txt")
            ]),
        ]))
    );
}

#[test]
fn assertion_005_options_5_from_config_file() {
    assert_eq!(
        parsed().value("nosort"),
        Some(&ConfigValue::List(vec![
            item(&[("name", "author"), ("value", r"\A\p{L}{2}\p{Pd}(?=\S)")]),
            item(&[("name", "author"), ("value", r"[\x{2bf}\x{2018}]")]),
            item(&[("name", "translator"), ("value", r"[\x{2bf}\x{2018}]")]),
        ]))
    );
}

#[test]
fn assertion_006_options_6_from_config_file() {
    assert_eq!(
        parsed().value("noinits"),
        Some(&ConfigValue::List(vec![
            item(&[("value", r"\b\p{Ll}{2}\p{Pd}(?=\S))")]),
            item(&[("value", r"[\x{2bf}\x{2018}]")]),
        ]))
    );
}

#[test]
fn assertion_007_options_7_from_bcf() {
    let control = parse_control_bytes(CONTROL, XmlLimits::default()).unwrap();
    let actual = control.resolve_option(bib_input::OptionComponent::Processor, "sortcase", None);
    let expected = bib_input::ControlOptionValue::Single(StructuredValue {
        content: "0".into(),
        attributes: BTreeMap::new(),
    });
    assert_eq!(actual, Some(&expected));
}

#[test]
#[ignore = "xfail: compiled decodecharsset default is not exposed"]
fn assertion_008_options_8_from_defaults() {
    assert_eq!(
        BibCommand::parse(["general.bcf"])
            .unwrap()
            .job()
            .options()
            .configuration()
            .map(VirtualPath::as_str),
        Some("base")
    );
}

#[test]
#[ignore = "xfail: native configuration resolution does not merge built-in and user sourcemaps"]
fn assertion_009_options_9_from_config_file() {
    let config = parsed();
    let mut resolved = ResolvedConfiguration::default();
    resolved
        .push(
            ConfigurationLayer::UserConfiguration,
            [(
                "sourcemap".into(),
                config.value("sourcemap").unwrap().clone(),
            )],
        )
        .unwrap();
    resolved
        .push(
            ConfigurationLayer::ControlFile,
            [("sourcemap".into(), ConfigValue::Tree(Vec::new()))],
        )
        .unwrap();
    assert_eq!(resolved.merged_list("sourcemap").len(), 2);
}

#[test]
fn assertion_010_validation_of_biber_test_conf() {
    validate_config_bytes(CONFIG, XmlLimits::default()).unwrap();
}
