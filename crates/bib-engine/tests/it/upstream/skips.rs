//! Native translations of upstream `t/skips.t` at commit 74252e6.

use bib_engine::{FieldId, FieldValue};

use super::maps::{entry, output_entry, run_fixture, text_field};

const EXPECTED_SET1: &str = r#"    \entry{seta}{set}{}{}
      \set{set:membera,set:memberb,set:memberc}
      \field{labelalpha}{Doe10}
      \field{extraalpha}{1}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \keyw{key1,key2}
    \endentry
"#;

const EXPECTED_SET2: &str = r#"    \entry{set:membera}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{seta}
      \name{author}{1}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
      }
      \strng{namehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{bibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorbibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authornamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Set Member A}
      \field{year}{2010}
      \field{dateera}{ce}
      \keyw{key1,key2}
    \endentry
"#;

const EXPECTED_SET3: &str = r#"    \entry{set:memberb}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{seta}
      \name{author}{1}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
      }
      \strng{namehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{bibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorbibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authornamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Set Member B}
      \field{year}{2010}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_SET4: &str = r#"    \entry{set:memberc}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{seta}
      \name{author}{1}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
      }
      \strng{namehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{bibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorbibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authornamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Set Member C}
      \field{year}{2010}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_NOSET1: &str = r#"    \entry{noseta}{book}{}{}
      \name{author}{1}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
      }
      \strng{namehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{bibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorbibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authornamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \field{extraname}{3}
      \field{labelalpha}{Doe10}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{2}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Stand-Alone A}
      \field{year}{2010}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_NOSET2: &str = r#"    \entry{nosetb}{book}{}{}
      \name{author}{1}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
      }
      \strng{namehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{bibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorbibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authornamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \field{extraname}{4}
      \field{labelalpha}{Doe10}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{3}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{3}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Stand-Alone B}
      \field{year}{2010}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_NOSET3: &str = r#"    \entry{nosetc}{book}{}{}
      \name{author}{1}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
      }
      \strng{namehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{bibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorbibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authornamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \field{extraname}{5}
      \field{labelalpha}{Doe10}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{4}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{4}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Stand-Alone C}
      \field{year}{2010}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_SK4: &str = r#"    \entry{skip4}{article}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \name{author}{1}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
      }
      \list{location}{1}{%
        {Cambridge}%
      }
      \list{publisher}{1}{%
        {A press}%
      }
      \strng{namehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{fullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{bibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorbibnamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authornamehash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhash}{bd051a2f7a5f377e3a62581b0e0f8577}
      \strng{authorfullhashraw}{bd051a2f7a5f377e3a62581b0e0f8577}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{shorthand}{AWS}
      \field{title}{Algorithms Which Sort}
      \field{year}{1932}
    \endentry
"#;

#[test]
#[ignore = "xfail: per-entry Biber skip options are not represented by bib-engine"]
fn assertion_001_passing_skipbib_through() {
    let result = run_fixture("skips");
    let value = entry(&result, 0, "skip1")
        .and_then(|entry| entry.fields().get(&FieldId::new("options").unwrap()));
    assert!(
        matches!(value, Some(FieldValue::LiteralList(values)) if values.iter().map(|value| value.as_str()).eq(["skipbib"]))
    );
}

#[test]
#[ignore = "xfail: Biber labelalpha derivation is not implemented by bib-engine"]
fn assertion_002_normal_labelalpha() {
    let result = run_fixture("skips");
    assert_eq!(
        entry(&result, 0, "skip2").and_then(|entry| text_field(entry, "labelalpha")),
        Some("SA")
    );
}

#[test]
fn assertion_003_normal_labelyear() {
    let result = run_fixture("skips");
    let value = entry(&result, 0, "skip2")
        .and_then(|entry| entry.fields().get(&FieldId::new("year").unwrap()));
    assert!(matches!(value, Some(FieldValue::Integer(1995))));
}

#[test]
fn assertion_004_skiplab_no_labelalpha() {
    let result = run_fixture("skips");
    assert_eq!(
        entry(&result, 0, "skip3").and_then(|entry| text_field(entry, "labelalpha")),
        None
    );
}

#[test]
fn assertion_006_dataonly_no_labelalpha() {
    let result = run_fixture("skips");
    assert_eq!(
        entry(&result, 0, "skip4").and_then(|entry| text_field(entry, "labelalpha")),
        None
    );
}

#[test]
fn assertion_005_skiplab_no_labelyear() {
    let result = run_fixture("skips");
    assert_eq!(
        entry(&result, 0, "skip3").and_then(|entry| text_field(entry, "labeldatesource")),
        Some("")
    );
}

#[test]
fn assertion_008_dataonly_no_labelyear() {
    let result = run_fixture("skips");
    assert_eq!(
        entry(&result, 0, "skip4").and_then(|entry| text_field(entry, "labeldatesource")),
        Some("")
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_007_dataonly_checking_output() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "skip4").as_deref(),
        Some(EXPECTED_SK4)
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_009_set_parent_with_labels() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "seta").as_deref(),
        Some(EXPECTED_SET1)
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_010_set_member_no_labels_1() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "set:membera").as_deref(),
        Some(EXPECTED_SET2)
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_011_set_member_no_labels_2() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "set:memberb").as_deref(),
        Some(EXPECTED_SET3)
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_012_set_member_no_labels_3() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "set:memberc").as_deref(),
        Some(EXPECTED_SET4)
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_013_not_a_set_member_extradate_continues_from_set_1() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "noseta").as_deref(),
        Some(EXPECTED_NOSET1)
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_014_not_a_set_member_extradate_continues_from_set_2() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "nosetb").as_deref(),
        Some(EXPECTED_NOSET2)
    );
}

#[test]
#[ignore = "xfail: Biber skip/set output parity is not implemented by bib-engine"]
fn assertion_015_not_a_set_member_extradate_continues_from_set_3() {
    let result = run_fixture("skips");
    assert_eq!(
        output_entry(&result, "nosetc").as_deref(),
        Some(EXPECTED_NOSET3)
    );
}
