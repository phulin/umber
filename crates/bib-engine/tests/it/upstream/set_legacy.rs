//! Native translations of upstream `t/set-legacy.t` at commit 74252e6.

use super::maps::{output_entry, run_fixture};

const EXPECTED_1: &str = r#"    \entry{Elias1955}{set}{}{}
      \set{Elias1955a,Elias1955b}
      \field{sortinit}{1}
      \field{sortinithash}{4f6aaa89bab872aa0999fec09ff8e98a}
    \endentry
"#;

const EXPECTED_2: &str = r#"    \entry{Elias1955a}{article}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{Elias1955}
      \name{author}{1}{}{%
        {{hash=68f587f427e068e26043d54745351d58}{%
           family={Elias},
           familyi={E\bibinitperiod},
           given={P.},
           giveni={P\bibinitperiod}}}%
      }
      \strng{namehash}{68f587f427e068e26043d54745351d58}
      \strng{fullhash}{68f587f427e068e26043d54745351d58}
      \strng{fullhashraw}{68f587f427e068e26043d54745351d58}
      \strng{bibnamehash}{68f587f427e068e26043d54745351d58}
      \strng{authorbibnamehash}{68f587f427e068e26043d54745351d58}
      \strng{authornamehash}{68f587f427e068e26043d54745351d58}
      \strng{authorfullhash}{68f587f427e068e26043d54745351d58}
      \strng{authorfullhashraw}{68f587f427e068e26043d54745351d58}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{issn}{0096-1000}
      \field{journaltitle}{IRE Transactions on Information Theory}
      \field{month}{3}
      \field{number}{1}
      \field{title}{Predictive coding--I}
      \field{volume}{1}
      \field{year}{1955}
      \field{pages}{16\bibrangedash 24}
      \range{pages}{9}
      \verb{doi}
      \verb 10.1109/TIT.1955.1055126
      \endverb
      \warn{\item Field 'entryset' is no longer needed in set member entries in Biber - ignoring in entry 'Elias1955a'}
    \endentry
"#;

const EXPECTED_3: &str = r#"    \entry{Elias1955b}{article}{skipbib=true,skipbiblist=true,skiplab=true,uniquelist=false,uniquename=false}{}
      \inset{Elias1955}
      \name{author}{1}{}{%
        {{hash=68f587f427e068e26043d54745351d58}{%
           family={Elias},
           familyi={E\bibinitperiod},
           given={P.},
           giveni={P\bibinitperiod}}}%
      }
      \strng{namehash}{68f587f427e068e26043d54745351d58}
      \strng{fullhash}{68f587f427e068e26043d54745351d58}
      \strng{fullhashraw}{68f587f427e068e26043d54745351d58}
      \strng{bibnamehash}{68f587f427e068e26043d54745351d58}
      \strng{authorbibnamehash}{68f587f427e068e26043d54745351d58}
      \strng{authornamehash}{68f587f427e068e26043d54745351d58}
      \strng{authorfullhash}{68f587f427e068e26043d54745351d58}
      \strng{authorfullhashraw}{68f587f427e068e26043d54745351d58}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{issn}{0096-1000}
      \field{journaltitle}{IRE Transactions on Information Theory}
      \field{month}{3}
      \field{number}{1}
      \field{title}{Predictive coding--II}
      \field{volume}{1}
      \field{year}{1955}
      \field{pages}{24\bibrangedash 33}
      \range{pages}{10}
      \verb{doi}
      \verb 10.1109/TIT.1955.1055116
      \endverb
      \warn{\item Field 'entryset' is no longer needed in set member entries in Biber - ignoring in entry 'Elias1955b'}
    \endentry
"#;

#[test]
#[ignore = "xfail: legacy Biber entry-set processing is not implemented by bib-engine"]
fn assertion_001_legacy_set_test_1() {
    let result = run_fixture("set-legacy");
    assert_eq!(
        output_entry(&result, "Elias1955").as_deref(),
        Some(EXPECTED_1)
    );
}

#[test]
#[ignore = "xfail: legacy Biber entry-set processing is not implemented by bib-engine"]
fn assertion_002_legacy_set_test_2() {
    let result = run_fixture("set-legacy");
    assert_eq!(
        output_entry(&result, "Elias1955a").as_deref(),
        Some(EXPECTED_2)
    );
}

#[test]
#[ignore = "xfail: legacy Biber entry-set processing is not implemented by bib-engine"]
fn assertion_003_legacy_set_test_3() {
    let result = run_fixture("set-legacy");
    assert_eq!(
        output_entry(&result, "Elias1955b").as_deref(),
        Some(EXPECTED_3)
    );
}
