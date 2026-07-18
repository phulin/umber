//! Native translations of upstream `t/sort-complex.t` at commit 74252e6.

use super::maps::{list_keys, output_entry, run_fixture};

const EXPECTED_L4: &str = r#"    \entry{L4}{book}{}{}
      \true{moreauthor}
      \true{morelabelname}
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
        {Another press}%
      }
      \strng{namehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{fullhash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{fullhashraw}{6eb389989020e8246fee90ac93fcecbe}
      \strng{bibnamehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authorbibnamehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authornamehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authorfullhash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authorfullhashraw}{6eb389989020e8246fee90ac93fcecbe}
      \field{extraname}{2}
      \field{labelalpha}{Doe\textbf{+}95}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extraalpha}{2}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Some title about sorting}
      \field{year}{1995}
    \endentry
"#;

const EXPECTED_L1: &str = r#"    \entry{L1}{book}{}{}
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
      \field{extraname}{1}
      \field{labelalpha}{Doe95}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Algorithms For Sorting}
      \field{year}{1995}
    \endentry
"#;

const EXPECTED_L2: &str = r#"    \entry{L2}{book}{}{}
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
      \field{extraname}{3}
      \field{labelalpha}{Doe95}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extraalpha}{3}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Sorting Algorithms}
      \field{year}{1995}
    \endentry
"#;

const EXPECTED_L3: &str = r#"    \entry{L3}{book}{}{}
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
      \field{extraname}{2}
      \field{labelalpha}{Doe95}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extraalpha}{2}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{More and More Algorithms}
      \field{year}{1995}
    \endentry
"#;

const EXPECTED_L5: &str = r#"    \entry{L5}{book}{}{}
      \true{moreauthor}
      \true{morelabelname}
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
        {Another press}%
      }
      \strng{namehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{fullhash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{fullhashraw}{6eb389989020e8246fee90ac93fcecbe}
      \strng{bibnamehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authorbibnamehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authornamehash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authorfullhash}{6eb389989020e8246fee90ac93fcecbe}
      \strng{authorfullhashraw}{6eb389989020e8246fee90ac93fcecbe}
      \field{extraname}{1}
      \field{labelalpha}{Doe\textbf{+}95}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Some other title about sorting}
      \field{year}{1995}
    \endentry
"#;

#[test]
#[ignore = "xfail: Biber sorting-template semantics are not implemented by bib-engine"]
fn assertion_001_sort_template() {
    let result = run_fixture("sort-complex");
    assert_eq!(
        list_keys(&result, 0, "nyt/global//global/global/global"),
        ["L5", "L4", "L1", "L3", "L2"]
    );
}

#[test]
#[ignore = "xfail: Biber complex-sort output fields are not implemented by bib-engine"]
fn assertion_002_alphaothers_set_by_and_others() {
    let result = run_fixture("sort-complex");
    assert_eq!(output_entry(&result, "L4").as_deref(), Some(EXPECTED_L4));
}

#[test]
#[ignore = "xfail: Biber complex-sort output fields are not implemented by bib-engine"]
fn assertion_003_bbl_test_1() {
    let result = run_fixture("sort-complex");
    assert_eq!(output_entry(&result, "L1").as_deref(), Some(EXPECTED_L1));
}

#[test]
#[ignore = "xfail: Biber complex-sort output fields are not implemented by bib-engine"]
fn assertion_004_bbl_test_2() {
    let result = run_fixture("sort-complex");
    assert_eq!(output_entry(&result, "L2").as_deref(), Some(EXPECTED_L2));
}

#[test]
#[ignore = "xfail: Biber complex-sort output fields are not implemented by bib-engine"]
fn assertion_005_bbl_test_3() {
    let result = run_fixture("sort-complex");
    assert_eq!(output_entry(&result, "L3").as_deref(), Some(EXPECTED_L3));
}

#[test]
#[ignore = "xfail: Biber complex-sort output fields are not implemented by bib-engine"]
fn assertion_006_bbl_test_4() {
    let result = run_fixture("sort-complex");
    assert_eq!(output_entry(&result, "L5").as_deref(), Some(EXPECTED_L5));
}

#[test]
#[ignore = "xfail: Biber complex sorting/source-map reset is not implemented by bib-engine"]
fn assertion_007_sortorder_1() {
    let result = run_fixture("sort-complex");
    assert_eq!(
        list_keys(&result, 0, "nyt/global//global/global/global"),
        ["L5", "L4", "L1", "L3", "L2"]
    );
}

#[test]
fn assertion_008_sortorder_2() {
    let result = run_fixture("sort-complex");
    assert!(list_keys(&result, 0, "shorthand/global//global/global/global").is_empty());
}

#[test]
#[ignore = "xfail: Biber complex sorting/source-map reset is not implemented by bib-engine"]
fn assertion_009_sortorder_3() {
    let result = run_fixture("sort-complex");
    assert_eq!(
        list_keys(&result, 0, "shorthand/global//global/global/global"),
        ["L1", "L2", "L3", "L4", "L5"]
    );
}
