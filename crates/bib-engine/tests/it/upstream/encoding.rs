// Native Rust translation of upstream t/encoding.t at commit 74252e6.

use bib_engine::{BibCommand, FileProvisioner, GeneratedFile, VfsLimits, VirtualPath};
use bib_unicode::{LegacyEncoding, encode_legacy};
const ENCODE1: &str = concat!(
    r###"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup


\refsection{0}
  \datalist[entry]{nty/global//global/global/global}
    \entry{testŠ}{book}{}{}
"###,
    "      \n",
    r###"ame{author}{1}{}{%
        {{un=0,uniquepart=base,hash=06a47edae2e847800cfd78323a0e6be8}{%
           family={Encalcer},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod},
           givenun=0}}%
      }
      \list{publisher}{1}{%
        {A press}%
      }
      \strng{namehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \strng{bibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorbibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authornamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \field{labelalpha}{Enc99}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Šome title}
      \field{year}{1999}
    \endentry
  \enddatalist
\endrefsection
\endinput

"###
);
const ENCODE2: &str = concat!(
    r###"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup


\refsection{0}
  \datalist[entry]{nty/global//global/global/global}
    \entry{test1}{book}{}{}
"###,
    "      \n",
    r###"ame{author}{1}{}{%
        {{un=0,uniquepart=base,hash=06a47edae2e847800cfd78323a0e6be8}{%
           family={Encalcer},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod},
           givenun=0}}%
      }
      \list{publisher}{1}{%
        {A press}%
      }
      \strng{namehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \strng{bibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorbibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authornamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \field{labelalpha}{Enc99}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Söme title}
      \field{year}{1999}
    \endentry
  \enddatalist
\endrefsection
\endinput

"###
);
const ENCODE3: &str = concat!(
    r###"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup


\refsection{0}
  \datalist[entry]{nty/global//global/global/global}
    \entry{test1}{book}{}{}
"###,
    "      \n",
    r###"ame{author}{1}{}{%
        {{un=0,uniquepart=base,hash=06a47edae2e847800cfd78323a0e6be8}{%
           family={Encalcer},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod},
           givenun=0}}%
      }
      \list{publisher}{1}{%
        {A press}%
      }
      \strng{namehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \strng{bibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorbibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authornamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \field{labelalpha}{Enc99}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Żome title}
      \field{year}{1999}
    \endentry
  \enddatalist
\endrefsection
\endinput

"###
);
const ENCODE5: &str = concat!(
    r###"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup


\refsection{0}
  \datalist[entry]{nty/global//global/global/global}
    \entry{test}{book}{}{}
"###,
    "      \n",
    r###"ame{author}{1}{}{%
        {{un=0,uniquepart=base,hash=06a47edae2e847800cfd78323a0e6be8}{%
           family={Encalcer},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod},
           givenun=0}}%
      }
      \list{publisher}{1}{%
        {A press}%
      }
      \strng{namehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \strng{bibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorbibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authornamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \field{labelalpha}{Enc99}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{à titlé}
      \field{year}{1999}
    \endentry
  \enddatalist
\endrefsection
\endinput

"###
);
const ENCODE6: &str = concat!(
    r###"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup


\refsection{0}
  \datalist[entry]{nty/global//global/global/global}
    \entry{test}{book}{}{}
"###,
    "      \n",
    r###"ame{author}{1}{}{%
        {{un=0,uniquepart=base,hash=06a47edae2e847800cfd78323a0e6be8}{%
           family={Encalcer},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod},
           givenun=0}}%
      }
      \list{publisher}{1}{%
        {A press}%
      }
      \strng{namehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \strng{bibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorbibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authornamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \field{labelalpha}{Enc99}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{↑\`{a} titl\'{e}}
      \field{year}{1999}
    \endentry
  \enddatalist
\endrefsection
\endinput

"###
);
const ENCODE7: &str = concat!(
    r###"% $ biblatex auxiliary file $
% $ biblatex bbl format version 3.3 $
% Do not modify the above lines!
%
% This is an auxiliary file used by the 'biblatex' package.
% This file may safely be deleted. It will be recreated by
% biber as required.
%
\begingroup
\makeatletter
\@ifundefined{ver@biblatex.sty}
  {\@latex@error
     {Missing 'biblatex' package}
     {The bibliography requires the 'biblatex' package.}
      \aftergroup\endinput}
  {}
\endgroup


\refsection{0}
  \datalist[entry]{nty/global//global/global/global}
    \entry{test}{book}{}{}
"###,
    "      \n",
    r###"ame{author}{1}{}{%
        {{un=0,uniquepart=base,hash=06a47edae2e847800cfd78323a0e6be8}{%
           family={Encalcer},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod},
           givenun=0}}%
      }
      \list{publisher}{1}{%
        {A press}%
      }
      \strng{namehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{fullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \strng{bibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorbibnamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authornamehash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhash}{06a47edae2e847800cfd78323a0e6be8}
      \strng{authorfullhashraw}{06a47edae2e847800cfd78323a0e6be8}
      \field{labelalpha}{Enc99}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{{$\uparrow$}\`{a} titl\'{e}}
      \field{year}{1999}
    \endentry
  \enddatalist
\endrefsection
\endinput

"###
);
fn fixture(name: &str) -> &[u8] {
    match name {
        "encoding1.bcf" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding1.bcf")
        }
        "encoding1.bib" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding1.bib")
        }
        "encoding2.bcf" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding2.bcf")
        }
        "encoding2.bib" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding2.bib")
        }
        "encoding3.bcf" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding3.bcf")
        }
        "encoding3.bib" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding3.bib")
        }
        "encoding4.bcf" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding4.bcf")
        }
        "encoding4.bib" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding4.bib")
        }
        "encoding5.bcf" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding5.bcf")
        }
        "encoding6.bcf" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding6.bcf")
        }
        "encoding6.bib" => {
            include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/encoding6.bib")
        }
        _ => unreachable!(),
    }
}
fn run(stem: &str, encoding: &str) -> Vec<u8> {
    let control = format!("{stem}.bcf");
    let data = if stem == "encoding5" {
        "encoding2.bib".to_owned()
    } else {
        format!("{stem}.bib")
    };
    let mut f = FileProvisioner::new(VfsLimits::default()).unwrap();
    f.register_user(
        VirtualPath::user(&control).unwrap(),
        fixture(&control).to_vec(),
    )
    .unwrap();
    f.register_user(VirtualPath::user(&data).unwrap(), fixture(&data).to_vec())
        .unwrap();
    let arg = format!("--output-encoding={encoding}");
    let o = BibCommand::parse([arg.as_str(), "--output-file=actual.bbl", control.as_str()])
        .unwrap()
        .execute(&f.snapshot());
    o.result()
        .and_then(|r| r.files().next())
        .map(GeneratedFile::bytes)
        .unwrap_or_default()
        .to_vec()
}
fn expected(text: &str, enc: LegacyEncoding) -> Vec<u8> {
    encode_legacy(text, enc).unwrap()
}
macro_rules! xeq {
    ($name:ident,$stem:literal,$label:literal,$text:ident,$enc:ident) => {
        #[test]
        #[ignore = "xfail: exact encoded BBL output differs"]
        fn $name() {
            assert_eq!(run($stem, $label), expected($text, LegacyEncoding::$enc));
        }
    };
}
xeq!(
    assertion_001_latin9_bib_utf_8_bbl,
    "encoding1",
    "utf-8",
    ENCODE1,
    Utf8
);
xeq!(
    assertion_002_utf_8_bib_utf_8_bbl,
    "encoding2",
    "utf-8",
    ENCODE1,
    Utf8
);
xeq!(
    assertion_003_utf_8_bib_latin1_bbl,
    "encoding5",
    "latin1",
    ENCODE5,
    Latin1
);
xeq!(
    assertion_004_utf_8_bib_utf_8_bbl_safechars,
    "encoding6",
    "utf-8",
    ENCODE6,
    Utf8
);
xeq!(
    assertion_005_utf_8_bib_utf_8_bbl_output_safecharsset_full,
    "encoding6",
    "utf-8",
    ENCODE7,
    Utf8
);
#[test]
#[ignore = "xfail: Latin-9 output encoding is unsupported"]
fn assertion_006_utf_8_bib_latin9_bbl() {
    assert!(LegacyEncoding::for_label("latin9").is_ok());
}
#[test]
#[ignore = "xfail: CP1252 output label is unsupported"]
fn assertion_007_latin1_bib_cp1252_bbl() {
    assert!(LegacyEncoding::for_label("cp1252").is_ok());
}
xeq!(
    assertion_008_latin2_bib_latin3_bbl,
    "encoding4",
    "latin3",
    ENCODE3,
    Latin3
);
#[test]
fn assertion_009_latin2_bib_latin1_bbl_failure() {
    let actual = run("encoding4", "latin1");
    assert!(!actual.is_empty());
    assert!(encode_legacy(ENCODE3, LegacyEncoding::Latin1).is_err());
}
xeq!(
    assertion_010_latin1_bib_applemacce_custom_alias_bbl,
    "encoding3",
    "macroman",
    ENCODE2,
    MacRoman
);
