//! Native translations of upstream `t/skipsg.t` at commit 74252e6.

use super::maps::{output_entry, run_fixture};

const EXPECTED_S1: &str = r#"    \entry{S1}{book}{skipbib=false,skipbiblist=false,skiplab=false}{}
      \name{author}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{1}
      \field{labelalpha}{DA95}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{1}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title 1}
      \field{year}{1995}
    \endentry
"#;

const EXPECTED_S2: &str = r#"    \entry{S2}{book}{skipbib=false,skiplab=false}{}
      \name{author}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{extraname}{2}
      \field{labelalpha}{DA95}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{extradate}{2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{2}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title 2}
      \field{year}{1995}
    \endentry
"#;

const EXPECTED_S3: &str = r#"    \entry{S3}{book}{}{}
      \name{author}{2}{}{%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=df9bf04cd41245e6d23ad7543e7fd90d}{%
           family={Abrahams},
           familyi={A\bibinitperiod},
           given={Albert},
           giveni={A\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Oxford}%
      }
      \strng{namehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{fullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \strng{bibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorbibnamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authornamehash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhash}{8c77336299b25bdada7bf8038f46722f}
      \strng{authorfullhashraw}{8c77336299b25bdada7bf8038f46722f}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title 3}
      \field{year}{1995}
    \endentry
"#;

#[test]
#[ignore = "xfail: global Biber skip options are not implemented by bib-engine"]
fn assertion_001_global_skips_with_entry_override_1() {
    let result = run_fixture("skipsg");
    assert_eq!(output_entry(&result, "S1").as_deref(), Some(EXPECTED_S1));
}

#[test]
#[ignore = "xfail: global Biber skip options are not implemented by bib-engine"]
fn assertion_002_global_skips_with_entry_override_2() {
    let result = run_fixture("skipsg");
    assert_eq!(output_entry(&result, "S2").as_deref(), Some(EXPECTED_S2));
}

#[test]
#[ignore = "xfail: global Biber skip options are not implemented by bib-engine"]
fn assertion_003_global_skips_with_entry_override_3() {
    let result = run_fixture("skipsg");
    assert_eq!(output_entry(&result, "S3").as_deref(), Some(EXPECTED_S3));
}
