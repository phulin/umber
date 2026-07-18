//! Native translations of upstream `t/set-dynamic.t` at commit 74252e6.

use super::maps::{output_entry_nth, run_fixture, section_entry_keys};

const EXPECTED_1: &str = r#"    \entry{DynSet}{set}{}{}
      \set{Dynamic1,Dynamic2,Dynamic3}
      \field{sortinit}{1}
      \field{sortinithash}{4f6aaa89bab872aa0999fec09ff8e98a}
    \endentry
"#;

const EXPECTED_2: &str = r#"    \entry{Dynamic1}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{DynSet}
      \name{author}{1}{}{%
        {{hash=252caa7921a061ca92087a1a52f15b78}{%
           family={Dynamism},
           familyi={D\bibinitperiod},
           given={Derek},
           giveni={D\bibinitperiod}}}%
      }
      \strng{namehash}{252caa7921a061ca92087a1a52f15b78}
      \strng{fullhash}{252caa7921a061ca92087a1a52f15b78}
      \strng{fullhashraw}{252caa7921a061ca92087a1a52f15b78}
      \strng{bibnamehash}{252caa7921a061ca92087a1a52f15b78}
      \strng{authorbibnamehash}{252caa7921a061ca92087a1a52f15b78}
      \strng{authornamehash}{252caa7921a061ca92087a1a52f15b78}
      \strng{authorfullhash}{252caa7921a061ca92087a1a52f15b78}
      \strng{authorfullhashraw}{252caa7921a061ca92087a1a52f15b78}
      \field{sortinit}{8}
      \field{sortinithash}{a231b008ebf0ecbe0b4d96dcc159445f}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{annotation}{Some Dynamic Note}
      \field{shorthand}{d1}
      \field{title}{Doing Daring Deeds}
      \field{year}{2002}
    \endentry
"#;

const EXPECTED_3: &str = r#"    \entry{Dynamic2}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{DynSet}
      \name{author}{1}{}{%
        {{hash=894a5fe6de820f5dcce84a65581667f4}{%
           family={Bunting},
           familyi={B\bibinitperiod},
           given={Brian},
           giveni={B\bibinitperiod}}}%
      }
      \strng{namehash}{894a5fe6de820f5dcce84a65581667f4}
      \strng{fullhash}{894a5fe6de820f5dcce84a65581667f4}
      \strng{fullhashraw}{894a5fe6de820f5dcce84a65581667f4}
      \strng{bibnamehash}{894a5fe6de820f5dcce84a65581667f4}
      \strng{authorbibnamehash}{894a5fe6de820f5dcce84a65581667f4}
      \strng{authornamehash}{894a5fe6de820f5dcce84a65581667f4}
      \strng{authorfullhash}{894a5fe6de820f5dcce84a65581667f4}
      \strng{authorfullhashraw}{894a5fe6de820f5dcce84a65581667f4}
      \field{sortinit}{9}
      \field{sortinithash}{0a5ebc79d83c96b6579069544c73c7d4}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{shorthand}{d2}
      \field{title}{Beautiful Birthdays}
      \field{year}{2010}
    \endentry
"#;

const EXPECTED_4: &str = r#"    \entry{Dynamic3}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{DynSet}
      \name{author}{1}{}{%
        {{hash=fc3cc97631ceaecdde2aee6cc60ab42b}{%
           family={Regardless},
           familyi={R\bibinitperiod},
           given={Roger},
           giveni={R\bibinitperiod}}}%
      }
      \strng{namehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{fullhash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{fullhashraw}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{bibnamehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authorbibnamehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authornamehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authorfullhash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authorfullhashraw}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \field{sortinit}{1}
      \field{sortinithash}{4f6aaa89bab872aa0999fec09ff8e98a}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{shorthand}{d3}
      \field{title}{Reckless Ravishings}
      \field{year}{2000}
    \endentry
"#;

const EXPECTED_5: &str = r#"    \entry{Dynamic3}{book}{}{}
      \name{author}{1}{}{%
        {{hash=fc3cc97631ceaecdde2aee6cc60ab42b}{%
           family={Regardless},
           familyi={R\bibinitperiod},
           given={Roger},
           giveni={R\bibinitperiod}}}%
      }
      \strng{namehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{fullhash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{fullhashraw}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{bibnamehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authorbibnamehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authornamehash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authorfullhash}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \strng{authorfullhashraw}{fc3cc97631ceaecdde2aee6cc60ab42b}
      \field{sortinit}{1}
      \field{sortinithash}{4f6aaa89bab872aa0999fec09ff8e98a}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{shorthand}{d3}
      \field{title}{Reckless Ravishings}
      \field{year}{2000}
    \endentry
"#;

#[test]
#[ignore = "xfail: dynamic Biber entry sets are not implemented by bib-engine"]
fn assertion_001_citekeys() {
    let result = run_fixture("set-dynamic");
    let mut keys = section_entry_keys(&result, 0);
    keys.sort_unstable_by_key(|key| key.to_lowercase());
    assert_eq!(
        keys,
        [
            "dynamic1",
            "dynamic2",
            "dynamic3",
            "dynset",
            "elias1955",
            "elias1955a",
            "elias1955b",
            "static1",
            "static2",
            "static3",
            "static4"
        ]
    );
}

#[test]
#[ignore = "xfail: dynamic Biber entry sets are not implemented by bib-engine"]
fn assertion_002_dynamic_set_test_1() {
    let result = run_fixture("set-dynamic");
    assert_eq!(
        output_entry_nth(&result, "DynSet", 0).as_deref(),
        Some(EXPECTED_1)
    );
}

#[test]
#[ignore = "xfail: dynamic Biber entry sets are not implemented by bib-engine"]
fn assertion_003_dynamic_set_test_2() {
    let result = run_fixture("set-dynamic");
    assert_eq!(
        output_entry_nth(&result, "Dynamic1", 0).as_deref(),
        Some(EXPECTED_2)
    );
}

#[test]
#[ignore = "xfail: dynamic Biber entry sets are not implemented by bib-engine"]
fn assertion_004_dynamic_set_test_3() {
    let result = run_fixture("set-dynamic");
    assert_eq!(
        output_entry_nth(&result, "Dynamic2", 0).as_deref(),
        Some(EXPECTED_3)
    );
}

#[test]
#[ignore = "xfail: dynamic Biber entry sets are not implemented by bib-engine"]
fn assertion_005_dynamic_set_test_4() {
    let result = run_fixture("set-dynamic");
    assert_eq!(
        output_entry_nth(&result, "Dynamic3", 0).as_deref(),
        Some(EXPECTED_4)
    );
}

#[test]
#[ignore = "xfail: dynamic Biber entry sets are not implemented by bib-engine"]
fn assertion_006_dynamic_set_test_5() {
    let result = run_fixture("set-dynamic");
    assert_eq!(
        output_entry_nth(&result, "Dynamic3", 1).as_deref(),
        Some(EXPECTED_5)
    );
}

#[test]
#[ignore = "xfail: dynamic Biber entry sets are not implemented by bib-engine"]
fn assertion_007_dynamic_set_skipbiblist_1() {
    let result = run_fixture("set-dynamic");
    assert_eq!(
        output_entry_nth(&result, "Dynamic1", 0).as_deref(),
        Some(EXPECTED_2)
    );
}
