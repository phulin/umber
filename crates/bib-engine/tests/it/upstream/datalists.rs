//! Native translations of upstream `t/datalists.t` at commit 74252e6.

use super::maps::{list_keys, output_entry, run_fixture};

const EXPECTED_K11: &str = r#"    \entry{K11}{book}{}{}
      \name{author}{1}{sortingnamekeytemplatename=snk1}{%
        {{hash=4edc280a0ef229f9c061e3b121b17482}{%
           family={Xanax},
           familyi={X\bibinitperiod},
           given={Xavier},
           giveni={X\bibinitperiod}}}%
      }
      \list{location}{1}{%
        {Moscow}%
      }
      \list{publisher}{1}{%
        {Publisher}%
      }
      \strng{namehash}{4edc280a0ef229f9c061e3b121b17482}
      \strng{fullhash}{4edc280a0ef229f9c061e3b121b17482}
      \strng{fullhashraw}{4edc280a0ef229f9c061e3b121b17482}
      \strng{bibnamehash}{4edc280a0ef229f9c061e3b121b17482}
      \strng{authorbibnamehash}{4edc280a0ef229f9c061e3b121b17482}
      \strng{authornamehash}{4edc280a0ef229f9c061e3b121b17482}
      \strng{authorfullhash}{4edc280a0ef229f9c061e3b121b17482}
      \strng{authorfullhashraw}{4edc280a0ef229f9c061e3b121b17482}
      \field{sortinit}{a}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{One}
      \field{year}{1983}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_K12: &str = r#"    \entry{K12}{book}{}{}
      \name{author}{1}{}{%
        {{sortingnamekeytemplatename=snk2,hash=a846a485fc9cbb59b0ebeedd6ac637e4}{%
           family={Allen},
           familyi={A\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod}}}%
      }
      \list{location}{1}{%
        {Moscow}%
      }
      \list{publisher}{1}{%
        {Publisher}%
      }
      \strng{namehash}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \strng{fullhash}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \strng{fullhashraw}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \strng{bibnamehash}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \strng{authorbibnamehash}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \strng{authornamehash}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \strng{authorfullhash}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \strng{authorfullhashraw}{a846a485fc9cbb59b0ebeedd6ac637e4}
      \field{sortinit}{Z}
      \field{sortinithash}{96892c0b0a36bb8557c40c49813d48b3}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Two}
      \field{year}{1983}
      \field{dateera}{ce}
    \endentry
"#;

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_001_list_name_order() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "lname"),
        [
            "K11", "K1", "K2", "K4", "K3", "K7", "K8", "K9", "K10", "K12", "K5", "K6"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_002_list_year_order() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "lyear"),
        [
            "K8", "K9", "K10", "K4", "K1", "K11", "K12", "K2", "K3", "K6", "K5", "K7"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_003_list_title_order() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "ltitle"),
        [
            "K1", "K7", "K8", "K9", "K4", "K10", "K2", "K11", "K6", "K5", "K12", "K3"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_004_list_name_order_filtered_1() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "lnamef1"),
        ["K11", "K2", "K4", "K12", "K5", "K6"]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_005_list_name_order_filtered_2() {
    let result = run_fixture("datalists");
    assert_eq!(list_keys(&result, 0, "lnamef2"), ["K4"]);
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_006_list_name_order_filtered_3() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "lnamef3"),
        ["K11", "K1", "K2", "K7", "K12", "K5", "K6"]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_007_list_name_order_filtered_4() {
    let result = run_fixture("datalists");
    assert_eq!(list_keys(&result, 0, "lnamef4"), ["K3"]);
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_008_list_name_order_filtered_5() {
    let result = run_fixture("datalists");
    assert_eq!(list_keys(&result, 0, "lnamef5"), ["K1", "K3"]);
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_009_list_name_order_swedish() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "lnameswe"),
        [
            "K11", "K1", "K2", "K4", "K3", "K7", "K8", "K9", "K10", "K12", "K6", "K5"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_010_list_title_order_spanish() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "ltitlespan"),
        [
            "K1", "K4", "K10", "K7", "K8", "K9", "K2", "K11", "K6", "K5", "K12", "K3"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_011_list_granular_locale_spanish() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "ltitleset"),
        [
            "K1", "K7", "K9", "K8", "K4", "K10", "K2", "K11", "K6", "K5", "K12", "K3"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_012_list_sorting_name_key_templates_1() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 0, "lname"),
        [
            "K11", "K1", "K2", "K4", "K3", "K7", "K5", "K8", "K9", "K10", "K12", "K6"
        ]
    );
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_013_datalist_output_1() {
    let result = run_fixture("datalists");
    assert_eq!(output_entry(&result, "K11").as_deref(), Some(EXPECTED_K11));
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_014_datalist_output_2() {
    let result = run_fixture("datalists");
    assert_eq!(output_entry(&result, "K12").as_deref(), Some(EXPECTED_K12));
}

#[test]
#[ignore = "xfail: Biber datalist selection/sorting is not implemented by bib-engine"]
fn assertion_015_list_dates() {
    let result = run_fixture("datalists");
    assert_eq!(
        list_keys(&result, 1, "ldates"),
        ["D3", "D2", "D1", "D5", "D6", "D7", "D4"]
    );
}
