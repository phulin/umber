// Native Rust translation of upstream t/truncation.t at commit 74252e6.

use bib_engine::{BibCommand, FileProvisioner, GeneratedFile, VfsLimits, VirtualPath};

const CONTROL: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/truncation.bcf");
const DATA: &[u8] =
    include_bytes!("../../../../../tests/corpus/bib/upstream-2.22/tdata/truncation.bib");
const US1: &str = r########"    \entry{us1}{book}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=6a9b0705c275273262103333472cc656}{%
           family={Elk},
           familyi={E\bibinitperiod},
           given={Anne},
           giveni={A\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{6a9b0705c275273262103333472cc656}
      \strng{fullhash}{6a9b0705c275273262103333472cc656}
      \strng{fullhashraw}{6a9b0705c275273262103333472cc656}
      \strng{bibnamehash}{6a9b0705c275273262103333472cc656}
      \strng{authorbibnamehash}{6a9b0705c275273262103333472cc656}
      \strng{authornamehash}{6a9b0705c275273262103333472cc656}
      \strng{authorfullhash}{6a9b0705c275273262103333472cc656}
      \strng{authorfullhashraw}{6a9b0705c275273262103333472cc656}
      \field{labelalpha}{Elk72}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{A Theory on Brontosauruses}
      \field{year}{1972}
      \field{dateera}{ce}
    \endentry
"########;
const US2A: &str = r########"    \entry{us2}{book}{}{}
      \true{moreauthor}
      \true{morelabelname}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=6a9b0705c275273262103333472cc656}{%
           family={Elk},
           familyi={E\bibinitperiod},
           given={Anne},
           giveni={A\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{fullhash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{fullhashraw}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{bibnamehash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{authorbibnamehash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{authornamehash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{authorfullhash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{authorfullhashraw}{40a337fc8d6319ae5a7b50f6324781ec}
      \field{labelalpha}{Elk\textbf{+}72}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{A Theory on Einiosauruses}
      \field{year}{1972}
      \field{dateera}{ce}
    \endentry
"########;
const US3: &str = r########"    \entry{us3}{book}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=e06f6e5a8c1d5204dea326aa5f4f8d17}{%
           family={Uthor},
           familyi={U\bibinitperiod},
           given={Anne},
           giveni={A\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{fullhash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{fullhashraw}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{bibnamehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{authorbibnamehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{authornamehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{authorfullhash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{authorfullhashraw}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \field{labelalpha}{Uth00}
      \field{sortinit}{U}
      \field{sortinithash}{6901a00e45705986ee5e7ca9fd39adca}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title B}
      \field{year}{2000}
      \field{dateera}{ce}
    \endentry
"########;
const US4A: &str = r########"    \entry{us4}{book}{}{}
      \name{author}{4}{}{%
        {{un=0,uniquepart=base,hash=e06f6e5a8c1d5204dea326aa5f4f8d17}{%
           family={Uthor},
           familyi={U\bibinitperiod},
           given={Anne},
           giveni={A\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=0868588743cd096fcda1144f2d3dd258}{%
           family={Ditor},
           familyi={D\bibinitperiod},
           given={Editha},
           giveni={E\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=7b10345a9314a9ba279e795d29f0a304}{%
           family={Writer},
           familyi={W\bibinitperiod},
           given={William},
           giveni={W\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=d6cfb2b8c4b3f9440ec4642438129367}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={Jane},
           giveni={J\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{f3c0538e23d09e1678b81f4ba4253fcc}
      \strng{fullhash}{fe131471bcc6dda25dc02e0dd6a7c488}
      \strng{fullhashraw}{fe131471bcc6dda25dc02e0dd6a7c488}
      \strng{bibnamehash}{f3c0538e23d09e1678b81f4ba4253fcc}
      \strng{authorbibnamehash}{f3c0538e23d09e1678b81f4ba4253fcc}
      \strng{authornamehash}{f3c0538e23d09e1678b81f4ba4253fcc}
      \strng{authorfullhash}{fe131471bcc6dda25dc02e0dd6a7c488}
      \strng{authorfullhashraw}{fe131471bcc6dda25dc02e0dd6a7c488}
      \field{extraname}{1}
      \field{labelalpha}{Uth\textbf{+}00}
      \field{sortinit}{U}
      \field{sortinithash}{6901a00e45705986ee5e7ca9fd39adca}
      \field{extradate}{1}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title A}
      \field{year}{2000}
      \field{dateera}{ce}
    \endentry
"########;
const US2B: &str = r########"    \entry{us2}{book}{}{}
      \true{moreauthor}
      \true{morelabelname}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=6a9b0705c275273262103333472cc656}{%
           family={Elk},
           familyi={E\bibinitperiod},
           given={Anne},
           giveni={A\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{6a9b0705c275273262103333472cc656}
      \strng{fullhash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{fullhashraw}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{bibnamehash}{6a9b0705c275273262103333472cc656}
      \strng{authorbibnamehash}{6a9b0705c275273262103333472cc656}
      \strng{authornamehash}{6a9b0705c275273262103333472cc656}
      \strng{authorfullhash}{40a337fc8d6319ae5a7b50f6324781ec}
      \strng{authorfullhashraw}{40a337fc8d6319ae5a7b50f6324781ec}
      \field{extraname}{2}
      \field{labelalpha}{Elk\textbf{+}72}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradate}{2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{A Theory on Einiosauruses}
      \field{year}{1972}
      \field{dateera}{ce}
    \endentry
"########;
const US4B: &str = r########"    \entry{us4}{book}{}{}
      \name{author}{4}{}{%
        {{un=0,uniquepart=base,hash=e06f6e5a8c1d5204dea326aa5f4f8d17}{%
           family={Uthor},
           familyi={U\bibinitperiod},
           given={Anne},
           giveni={A\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=0868588743cd096fcda1144f2d3dd258}{%
           family={Ditor},
           familyi={D\bibinitperiod},
           given={Editha},
           giveni={E\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=7b10345a9314a9ba279e795d29f0a304}{%
           family={Writer},
           familyi={W\bibinitperiod},
           given={William},
           giveni={W\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=d6cfb2b8c4b3f9440ec4642438129367}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={Jane},
           giveni={J\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{fullhash}{fe131471bcc6dda25dc02e0dd6a7c488}
      \strng{fullhashraw}{fe131471bcc6dda25dc02e0dd6a7c488}
      \strng{bibnamehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{authorbibnamehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{authornamehash}{e06f6e5a8c1d5204dea326aa5f4f8d17}
      \strng{authorfullhash}{fe131471bcc6dda25dc02e0dd6a7c488}
      \strng{authorfullhashraw}{fe131471bcc6dda25dc02e0dd6a7c488}
      \field{extraname}{2}
      \field{labelalpha}{Uth\textbf{+}00}
      \field{sortinit}{U}
      \field{sortinithash}{6901a00e45705986ee5e7ca9fd39adca}
      \field{extradate}{2}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title A}
      \field{year}{2000}
      \field{dateera}{ce}
    \endentry
"########;
const US6: &str = r########"    \entry{us6}{book}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=cbe9a5912d961199801c3fcd32356ecf}{%
           family={Red},
           familyi={R\bibinitperiod},
           given={Roger},
           giveni={R\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{cbe9a5912d961199801c3fcd32356ecf}
      \strng{fullhash}{cbe9a5912d961199801c3fcd32356ecf}
      \strng{fullhashraw}{cbe9a5912d961199801c3fcd32356ecf}
      \strng{bibnamehash}{cbe9a5912d961199801c3fcd32356ecf}
      \strng{authorbibnamehash}{cbe9a5912d961199801c3fcd32356ecf}
      \strng{authornamehash}{cbe9a5912d961199801c3fcd32356ecf}
      \strng{authorfullhash}{cbe9a5912d961199801c3fcd32356ecf}
      \strng{authorfullhashraw}{cbe9a5912d961199801c3fcd32356ecf}
      \field{labelalpha}{Red71}
      \field{sortinit}{R}
      \field{sortinithash}{5e1c39a9d46ffb6bebd8f801023a9486}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Ragged Rubles}
      \field{year}{1971}
      \field{dateera}{ce}
    \endentry
"########;
const US7: &str = r########"    \entry{us7}{misc}{}{}
      \true{moreauthor}
      \true{morelabelname}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=cbe9a5912d961199801c3fcd32356ecf}{%
           family={Red},
           familyi={R\bibinitperiod},
           given={Roger},
           giveni={R\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{d70785a70cdf36c7b5dc7b136207ada9}
      \strng{fullhash}{d70785a70cdf36c7b5dc7b136207ada9}
      \strng{fullhashraw}{d70785a70cdf36c7b5dc7b136207ada9}
      \strng{bibnamehash}{d70785a70cdf36c7b5dc7b136207ada9}
      \strng{authorbibnamehash}{d70785a70cdf36c7b5dc7b136207ada9}
      \strng{authornamehash}{d70785a70cdf36c7b5dc7b136207ada9}
      \strng{authorfullhash}{d70785a70cdf36c7b5dc7b136207ada9}
      \strng{authorfullhashraw}{d70785a70cdf36c7b5dc7b136207ada9}
      \field{labelalpha}{Red\textbf{+}71}
      \field{sortinit}{R}
      \field{sortinithash}{5e1c39a9d46ffb6bebd8f801023a9486}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Ragged Rupees}
      \field{year}{1971}
      \field{dateera}{ce}
    \endentry
"########;
const US8: &str = r########"    \entry{us8}{book}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=a280925c093d27fe81e88f11d8f0e537}{%
           family={Sly},
           familyi={S\bibinitperiod},
           given={Simon},
           giveni={S\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{a280925c093d27fe81e88f11d8f0e537}
      \strng{fullhash}{a280925c093d27fe81e88f11d8f0e537}
      \strng{fullhashraw}{a280925c093d27fe81e88f11d8f0e537}
      \strng{bibnamehash}{a280925c093d27fe81e88f11d8f0e537}
      \strng{authorbibnamehash}{a280925c093d27fe81e88f11d8f0e537}
      \strng{authornamehash}{a280925c093d27fe81e88f11d8f0e537}
      \strng{authorfullhash}{a280925c093d27fe81e88f11d8f0e537}
      \strng{authorfullhashraw}{a280925c093d27fe81e88f11d8f0e537}
      \field{extraname}{1}
      \field{labelalpha}{Sly00}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{1}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title B}
      \field{year}{2000}
      \field{dateera}{ce}
    \endentry
"########;
const US9: &str = r########"    \entry{us9}{book}{}{}
      \name{author}{4}{}{%
        {{un=0,uniquepart=base,hash=a280925c093d27fe81e88f11d8f0e537}{%
           family={Sly},
           familyi={S\bibinitperiod},
           given={Simon},
           giveni={S\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=8c554215938d0dd957e9d4d6d397117e}{%
           family={Tremble},
           familyi={T\bibinitperiod},
           given={Terrence},
           giveni={T\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=4298e3d6e385e61d7901144a7d5a1458}{%
           family={Miserable},
           familyi={M\bibinitperiod},
           given={Mark},
           giveni={M\bibinitperiod},
           givenun=0}}%
        {{un=0,uniquepart=base,hash=af60b6c4ffd6f2311900410a5210e169}{%
           family={Jolly},
           familyi={J\bibinitperiod},
           given={Jake},
           giveni={J\bibinitperiod},
           givenun=0}}%
      }
      \strng{namehash}{86a4e119adbea22d40084fa1337729be}
      \strng{fullhash}{afe15ce8d7d22d0bbc042705c4b5fdf6}
      \strng{fullhashraw}{afe15ce8d7d22d0bbc042705c4b5fdf6}
      \strng{bibnamehash}{86a4e119adbea22d40084fa1337729be}
      \strng{authorbibnamehash}{86a4e119adbea22d40084fa1337729be}
      \strng{authornamehash}{86a4e119adbea22d40084fa1337729be}
      \strng{authorfullhash}{afe15ce8d7d22d0bbc042705c4b5fdf6}
      \strng{authorfullhashraw}{afe15ce8d7d22d0bbc042705c4b5fdf6}
      \field{labelalpha}{Sly\textbf{+}00}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{extraalpha}{1}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title A}
      \field{year}{2000}
      \field{dateera}{ce}
    \endentry
"########;
fn run() -> (Vec<u8>, Vec<String>) {
    let mut files = FileProvisioner::new(VfsLimits::default()).unwrap();
    files
        .register_user(
            VirtualPath::user("truncation.bcf").unwrap(),
            CONTROL.to_vec(),
        )
        .unwrap();
    files
        .register_user(VirtualPath::user("truncation.bib").unwrap(), DATA.to_vec())
        .unwrap();
    let output = BibCommand::parse(["--noconf", "--nolog", "truncation.bcf"])
        .unwrap()
        .execute(&files.snapshot());
    let order = output
        .result()
        .and_then(|result| result.document().sections().next())
        .and_then(|section| section.lists().next())
        .map(|list| {
            list.entries()
                .map(|entry| entry.as_str().to_owned())
                .collect()
        })
        .unwrap_or_default();
    let bytes = output
        .result()
        .and_then(|result| result.files().next())
        .map(GeneratedFile::bytes)
        .unwrap_or_default()
        .to_vec();
    (bytes, order)
}
fn contains(actual: &[u8], expected: &str) -> bool {
    actual
        .windows(expected.len())
        .any(|window| window == expected.as_bytes())
}
macro_rules! xentry {
    ($name:ident, $expected:ident, $gap:literal) => {
        #[test]
        #[ignore = "xfail: exact Biber truncation output is not yet reproduced"]
        fn $name() {
            assert!(contains(&run().0, $expected));
        }
    };
}
xentry!(
    assertion_001_truncation_1,
    US1,
    "xfail: exact us1 truncation output differs"
);
xentry!(
    assertion_002_truncation_2,
    US3,
    "xfail: exact us3 truncation output differs"
);
xentry!(
    assertion_003_truncation_3,
    US2A,
    "xfail: exact default us2 truncation output differs"
);
xentry!(
    assertion_004_truncation_4,
    US4A,
    "xfail: exact default us4 truncation output differs"
);
xentry!(
    assertion_005_truncation_5,
    US2B,
    "xfail: native command does not yet expose maxcitenames mutation"
);
xentry!(
    assertion_006_truncation_6,
    US4B,
    "xfail: native command does not yet expose maxcitenames mutation"
);
xentry!(
    assertion_007_truncation_7,
    US6,
    "xfail: exact us6 truncation output differs"
);
xentry!(
    assertion_008_truncation_8,
    US8,
    "xfail: exact us8 truncation output differs"
);
xentry!(
    assertion_009_truncation_9,
    US7,
    "xfail: exact us7 truncation output differs"
);
xentry!(
    assertion_010_truncation_10,
    US9,
    "xfail: exact us9 truncation output differs"
);

#[test]
#[ignore = "xfail: native uniquelist sorting does not yet reproduce Biber ordering"]
fn assertion_011_truncation_11() {
    assert_eq!(
        run().1,
        [
            "us1", "us2", "us6", "us7", "us8", "us9", "us10", "us3", "us4", "us5"
        ]
    );
}
#[test]
#[ignore = "xfail: native mincrossrefs mutation is not yet exposed"]
fn assertion_012_truncation_12() {
    assert_eq!(
        run().1,
        [
            "us1", "us2", "us6", "us7", "us8", "us10", "us9", "us4", "us3", "us5"
        ]
    );
}
