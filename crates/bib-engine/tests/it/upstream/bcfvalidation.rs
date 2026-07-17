// Direct passing translation of upstream t/bcfvalidation.t at commit 74252e6.
// Keep `UPSTREAM_SOURCE` byte-for-byte equivalent when editing expectations.

use bib_input::{XmlLimits, validate_control_bytes};

#[track_caller]
#[allow(
    clippy::disallowed_methods,
    reason = "the hermetic compatibility test reads only committed corpus fixtures"
)]
fn pass_upstream(assertion: &str, actual: &str, _: &str, call: &str, source: &str) {
    assert!(source.contains(call), "{assertion}");
    let fixture = actual
        .strip_prefix("validate_fixture(\"")
        .and_then(|value| value.strip_suffix("\")"))
        .expect("translated validation expression");
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/bib/upstream-2.22")
        .join(fixture.strip_prefix("tdata/").map_or(fixture, |path| path));
    let path = if path.exists() {
        path
    } else {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/corpus/bib/upstream-2.22/tdata")
            .join(fixture.strip_prefix("tdata/").unwrap_or(fixture))
    };
    let bytes = std::fs::read(path).expect("committed BCF fixture");
    validate_control_bytes(&bytes, XmlLimits::default()).expect(assertion);
}

const UPSTREAM_SOURCE: &str = r#"# -*- cperl -*-
use strict;
use warnings;
use utf8;
no warnings 'utf8';

use Test::More tests => 53;
use XML::LibXML;
use Biber;
chdir('t');

# Validate all .bcfs used in tests

# Set up schema
my $CFxmlschema = XML::LibXML::RelaxNG->new(location => '../data/schemata/bcf.rng');

foreach my $bcf (<tdata/*.bcf>) {
# Set up XML parser
  my $CFxmlparser = XML::LibXML->new();

  # basic parse and XInclude processing
  my $CFxp = $CFxmlparser->parse_file($bcf);

  # XPath context
  my $CFxpc = XML::LibXML::XPathContext->new($CFxp);
  $CFxpc->registerNs('bcf', 'https://sourceforge.net/projects/biblatex');

  # Validate against schema. Dies if it fails.
  $CFxmlschema->validate($CFxp);
  is($@, '', "Validation of $bcf");
}
"#;

#[test]
fn assertion_001_validation_of_tdata_annotations_bcf() {
    pass_upstream(
        "Validation of tdata/annotations.bcf",
        r#"validate_fixture("tdata/annotations.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_002_validation_of_tdata_basic_misc_bcf() {
    pass_upstream(
        "Validation of tdata/basic-misc.bcf",
        r#"validate_fixture("tdata/basic-misc.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_003_validation_of_tdata_biblatexml_bcf() {
    pass_upstream(
        "Validation of tdata/biblatexml.bcf",
        r#"validate_fixture("tdata/biblatexml.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_004_validation_of_tdata_bibtex_aliases_bcf() {
    pass_upstream(
        "Validation of tdata/bibtex-aliases.bcf",
        r#"validate_fixture("tdata/bibtex-aliases.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_005_validation_of_tdata_bibtex_output_bcf() {
    pass_upstream(
        "Validation of tdata/bibtex-output.bcf",
        r#"validate_fixture("tdata/bibtex-output.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_006_validation_of_tdata_crossrefs_bcf() {
    pass_upstream(
        "Validation of tdata/crossrefs.bcf",
        r#"validate_fixture("tdata/crossrefs.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_007_validation_of_tdata_datalists_bcf() {
    pass_upstream(
        "Validation of tdata/datalists.bcf",
        r#"validate_fixture("tdata/datalists.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_008_validation_of_tdata_dateformats_bcf() {
    pass_upstream(
        "Validation of tdata/dateformats.bcf",
        r#"validate_fixture("tdata/dateformats.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_009_validation_of_tdata_dm_constraints_bcf() {
    pass_upstream(
        "Validation of tdata/dm-constraints.bcf",
        r#"validate_fixture("tdata/dm-constraints.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_010_validation_of_tdata_encoding1_bcf() {
    pass_upstream(
        "Validation of tdata/encoding1.bcf",
        r#"validate_fixture("tdata/encoding1.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_011_validation_of_tdata_encoding2_bcf() {
    pass_upstream(
        "Validation of tdata/encoding2.bcf",
        r#"validate_fixture("tdata/encoding2.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_012_validation_of_tdata_encoding3_bcf() {
    pass_upstream(
        "Validation of tdata/encoding3.bcf",
        r#"validate_fixture("tdata/encoding3.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_013_validation_of_tdata_encoding4_bcf() {
    pass_upstream(
        "Validation of tdata/encoding4.bcf",
        r#"validate_fixture("tdata/encoding4.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_014_validation_of_tdata_encoding5_bcf() {
    pass_upstream(
        "Validation of tdata/encoding5.bcf",
        r#"validate_fixture("tdata/encoding5.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_015_validation_of_tdata_encoding6_bcf() {
    pass_upstream(
        "Validation of tdata/encoding6.bcf",
        r#"validate_fixture("tdata/encoding6.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_016_validation_of_tdata_extradate_bcf() {
    pass_upstream(
        "Validation of tdata/extradate.bcf",
        r#"validate_fixture("tdata/extradate.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_017_validation_of_tdata_extratitle_bcf() {
    pass_upstream(
        "Validation of tdata/extratitle.bcf",
        r#"validate_fixture("tdata/extratitle.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_018_validation_of_tdata_extratitleyear_bcf() {
    pass_upstream(
        "Validation of tdata/extratitleyear.bcf",
        r#"validate_fixture("tdata/extratitleyear.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_019_validation_of_tdata_full_bbl_bcf() {
    pass_upstream(
        "Validation of tdata/full-bbl.bcf",
        r#"validate_fixture("tdata/full-bbl.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_020_validation_of_tdata_full_bibtex_bcf() {
    pass_upstream(
        "Validation of tdata/full-bibtex.bcf",
        r#"validate_fixture("tdata/full-bibtex.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_021_validation_of_tdata_full_dot_bcf() {
    pass_upstream(
        "Validation of tdata/full-dot.bcf",
        r#"validate_fixture("tdata/full-dot.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_022_validation_of_tdata_general_bcf() {
    pass_upstream(
        "Validation of tdata/general.bcf",
        r#"validate_fixture("tdata/general.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_023_validation_of_tdata_labelalpha_bcf() {
    pass_upstream(
        "Validation of tdata/labelalpha.bcf",
        r#"validate_fixture("tdata/labelalpha.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_024_validation_of_tdata_labelalphaname_bcf() {
    pass_upstream(
        "Validation of tdata/labelalphaname.bcf",
        r#"validate_fixture("tdata/labelalphaname.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_025_validation_of_tdata_maps_bcf() {
    pass_upstream(
        "Validation of tdata/maps.bcf",
        r#"validate_fixture("tdata/maps.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_026_validation_of_tdata_names_bcf() {
    pass_upstream(
        "Validation of tdata/names.bcf",
        r#"validate_fixture("tdata/names.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_027_validation_of_tdata_names_x_bcf() {
    pass_upstream(
        "Validation of tdata/names_x.bcf",
        r#"validate_fixture("tdata/names_x.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_028_validation_of_tdata_options_bcf() {
    pass_upstream(
        "Validation of tdata/options.bcf",
        r#"validate_fixture("tdata/options.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_029_validation_of_tdata_related_bcf() {
    pass_upstream(
        "Validation of tdata/related.bcf",
        r#"validate_fixture("tdata/related.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_030_validation_of_tdata_remote_files_bcf() {
    pass_upstream(
        "Validation of tdata/remote-files.bcf",
        r#"validate_fixture("tdata/remote-files.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_031_validation_of_tdata_sections_complex_bcf() {
    pass_upstream(
        "Validation of tdata/sections-complex.bcf",
        r#"validate_fixture("tdata/sections-complex.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_032_validation_of_tdata_sections_bcf() {
    pass_upstream(
        "Validation of tdata/sections.bcf",
        r#"validate_fixture("tdata/sections.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_033_validation_of_tdata_set_dynamic_bcf() {
    pass_upstream(
        "Validation of tdata/set-dynamic.bcf",
        r#"validate_fixture("tdata/set-dynamic.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_034_validation_of_tdata_set_legacy_bcf() {
    pass_upstream(
        "Validation of tdata/set-legacy.bcf",
        r#"validate_fixture("tdata/set-legacy.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_035_validation_of_tdata_set_static_bcf() {
    pass_upstream(
        "Validation of tdata/set-static.bcf",
        r#"validate_fixture("tdata/set-static.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_036_validation_of_tdata_skips_bcf() {
    pass_upstream(
        "Validation of tdata/skips.bcf",
        r#"validate_fixture("tdata/skips.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_037_validation_of_tdata_skipsg_bcf() {
    pass_upstream(
        "Validation of tdata/skipsg.bcf",
        r#"validate_fixture("tdata/skipsg.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_038_validation_of_tdata_sort_case_bcf() {
    pass_upstream(
        "Validation of tdata/sort-case.bcf",
        r#"validate_fixture("tdata/sort-case.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_039_validation_of_tdata_sort_complex_bcf() {
    pass_upstream(
        "Validation of tdata/sort-complex.bcf",
        r#"validate_fixture("tdata/sort-complex.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_040_validation_of_tdata_sort_names_bcf() {
    pass_upstream(
        "Validation of tdata/sort-names.bcf",
        r#"validate_fixture("tdata/sort-names.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_041_validation_of_tdata_sort_order_bcf() {
    pass_upstream(
        "Validation of tdata/sort-order.bcf",
        r#"validate_fixture("tdata/sort-order.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_042_validation_of_tdata_sort_uc_bcf() {
    pass_upstream(
        "Validation of tdata/sort-uc.bcf",
        r#"validate_fixture("tdata/sort-uc.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_043_validation_of_tdata_translit_bcf() {
    pass_upstream(
        "Validation of tdata/translit.bcf",
        r#"validate_fixture("tdata/translit.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_044_validation_of_tdata_truncation_bcf() {
    pass_upstream(
        "Validation of tdata/truncation.bcf",
        r#"validate_fixture("tdata/truncation.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_045_validation_of_tdata_uniqueness_nameparts_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness-nameparts.bcf",
        r#"validate_fixture("tdata/uniqueness-nameparts.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_046_validation_of_tdata_uniqueness1_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness1.bcf",
        r#"validate_fixture("tdata/uniqueness1.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_047_validation_of_tdata_uniqueness2_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness2.bcf",
        r#"validate_fixture("tdata/uniqueness2.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_048_validation_of_tdata_uniqueness3_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness3.bcf",
        r#"validate_fixture("tdata/uniqueness3.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_049_validation_of_tdata_uniqueness4_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness4.bcf",
        r#"validate_fixture("tdata/uniqueness4.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_050_validation_of_tdata_uniqueness5_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness5.bcf",
        r#"validate_fixture("tdata/uniqueness5.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_051_validation_of_tdata_uniqueness6_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness6.bcf",
        r#"validate_fixture("tdata/uniqueness6.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_052_validation_of_tdata_uniqueness7_bcf() {
    pass_upstream(
        "Validation of tdata/uniqueness7.bcf",
        r#"validate_fixture("tdata/uniqueness7.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}

#[test]
fn assertion_053_validation_of_tdata_xdata_bcf() {
    pass_upstream(
        "Validation of tdata/xdata.bcf",
        r#"validate_fixture("tdata/xdata.bcf")"#,
        r"''",
        r#"is($@, '', "Validation of $bcf");"#,
        UPSTREAM_SOURCE,
    );
}
