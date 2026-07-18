//! Native translations of upstream `t/set-static.t` at commit 74252e6.

use super::maps::{list_keys, output_entry_nth, run_fixture};

const EXPECTED_1: &str = r#"    \entry{Static1}{set}{}{}
      \set{Static2,Static4,Static3}
      \field{sortinit}{B}
      \field{sortinithash}{d7095fff47cda75ca2589920aae98399}
      \field{annotation}{Some notes}
      \field{shorthand}{STAT1}
    \endentry
"#;

const EXPECTED_2: &str = r#"    \entry{Static2}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{Static1}
      \name{author}{1}{}{%
        {{hash=43874d80d7ce68027102819f16c47df1}{%
           family={Bumble},
           familyi={B\bibinitperiod},
           given={Brian},
           giveni={B\bibinitperiod}}}%
      }
      \strng{namehash}{43874d80d7ce68027102819f16c47df1}
      \strng{fullhash}{43874d80d7ce68027102819f16c47df1}
      \strng{fullhashraw}{43874d80d7ce68027102819f16c47df1}
      \strng{bibnamehash}{43874d80d7ce68027102819f16c47df1}
      \strng{authorbibnamehash}{43874d80d7ce68027102819f16c47df1}
      \strng{authornamehash}{43874d80d7ce68027102819f16c47df1}
      \strng{authorfullhash}{43874d80d7ce68027102819f16c47df1}
      \strng{authorfullhashraw}{43874d80d7ce68027102819f16c47df1}
      \field{sortinit}{B}
      \field{sortinithash}{d7095fff47cda75ca2589920aae98399}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{annotation}{Some Blessed Note}
      \field{title}{Blessed Brains}
      \field{year}{2001}
    \endentry
"#;

const EXPECTED_3: &str = r#"    \entry{Static3}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{Static1}
      \name{author}{1}{}{%
        {{hash=22dafa5cd57bb5dd7f3e3bab98fd539c}{%
           family={Dingle},
           familyi={D\bibinitperiod},
           given={Derek},
           giveni={D\bibinitperiod}}}%
      }
      \strng{namehash}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \strng{fullhash}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \strng{fullhashraw}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \strng{bibnamehash}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \strng{authorbibnamehash}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \strng{authornamehash}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \strng{authorfullhash}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \strng{authorfullhashraw}{22dafa5cd57bb5dd7f3e3bab98fd539c}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Castles and Crime}
      \field{year}{2002}
    \endentry
"#;

const EXPECTED_4: &str = r#"    \entry{Static4}{book}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{Static1}
      \name{author}{1}{}{%
        {{hash=da80091c8cd89e5269bd55af1bd5d2fa}{%
           family={Crenellation},
           familyi={C\bibinitperiod},
           given={Clive},
           giveni={C\bibinitperiod}}}%
      }
      \strng{namehash}{da80091c8cd89e5269bd55af1bd5d2fa}
      \strng{fullhash}{da80091c8cd89e5269bd55af1bd5d2fa}
      \strng{fullhashraw}{da80091c8cd89e5269bd55af1bd5d2fa}
      \strng{bibnamehash}{da80091c8cd89e5269bd55af1bd5d2fa}
      \strng{authorbibnamehash}{da80091c8cd89e5269bd55af1bd5d2fa}
      \strng{authornamehash}{da80091c8cd89e5269bd55af1bd5d2fa}
      \strng{authorfullhash}{da80091c8cd89e5269bd55af1bd5d2fa}
      \strng{authorfullhashraw}{da80091c8cd89e5269bd55af1bd5d2fa}
      \field{sortinit}{C}
      \field{sortinithash}{4d103a86280481745c9c897c925753c0}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Dungeons, Dark and Dangerous}
      \field{year}{2005}
    \endentry
"#;

const EXPECTED_5: &str = r#"    \entry{Static2}{book}{}{}
      \name{author}{1}{}{%
        {{hash=43874d80d7ce68027102819f16c47df1}{%
           family={Bumble},
           familyi={B\bibinitperiod},
           given={Brian},
           giveni={B\bibinitperiod}}}%
      }
      \strng{namehash}{43874d80d7ce68027102819f16c47df1}
      \strng{fullhash}{43874d80d7ce68027102819f16c47df1}
      \strng{fullhashraw}{43874d80d7ce68027102819f16c47df1}
      \strng{bibnamehash}{43874d80d7ce68027102819f16c47df1}
      \strng{authorbibnamehash}{43874d80d7ce68027102819f16c47df1}
      \strng{authornamehash}{43874d80d7ce68027102819f16c47df1}
      \strng{authorfullhash}{43874d80d7ce68027102819f16c47df1}
      \strng{authorfullhashraw}{43874d80d7ce68027102819f16c47df1}
      \field{sortinit}{1}
      \field{sortinithash}{4f6aaa89bab872aa0999fec09ff8e98a}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{annotation}{Some Blessed Note}
      \field{title}{Blessed Brains}
      \field{year}{2001}
    \endentry
"#;

#[test]
#[ignore = "xfail: static Biber entry sets are not implemented by bib-engine"]
fn assertion_001_static_set_test_1() {
    let result = run_fixture("set-static");
    assert_eq!(
        output_entry_nth(&result, "Static1", 0).as_deref(),
        Some(EXPECTED_1)
    );
}

#[test]
#[ignore = "xfail: static Biber entry sets are not implemented by bib-engine"]
fn assertion_002_static_set_test_2() {
    let result = run_fixture("set-static");
    assert_eq!(
        output_entry_nth(&result, "Static2", 0).as_deref(),
        Some(EXPECTED_2)
    );
}

#[test]
#[ignore = "xfail: static Biber entry sets are not implemented by bib-engine"]
fn assertion_003_static_set_test_3() {
    let result = run_fixture("set-static");
    assert_eq!(
        output_entry_nth(&result, "Static3", 0).as_deref(),
        Some(EXPECTED_3)
    );
}

#[test]
#[ignore = "xfail: static Biber entry sets are not implemented by bib-engine"]
fn assertion_004_static_set_test_4() {
    let result = run_fixture("set-static");
    assert_eq!(
        output_entry_nth(&result, "Static4", 0).as_deref(),
        Some(EXPECTED_4)
    );
}

#[test]
#[ignore = "xfail: static Biber entry sets are not implemented by bib-engine"]
fn assertion_005_static_set_test_5() {
    let result = run_fixture("set-static");
    assert_eq!(
        output_entry_nth(&result, "Static2", 1).as_deref(),
        Some(EXPECTED_5)
    );
}

#[test]
#[ignore = "xfail: Biber set shorthand sorting is not implemented by bib-engine"]
fn assertion_006_shorthand_sets() {
    let result = run_fixture("set-static");
    assert_eq!(
        list_keys(
            &result,
            0,
            "shorthand:shorthand/global//global/global/global"
        ),
        ["Static2", "Static3", "Static4", "Static1"]
    );
}
