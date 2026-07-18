//! Native translations of upstream `t/crossrefs.t` at commit 74252e6.

use super::maps::{entry, output_entry, try_run_fixture};

const EXPECTED_CR1: &str = r#"    \entry{cr1}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=121b6dc164b5b619c81c670fbd823f12}{%
           family={Gullam},
           familyi={G\bibinitperiod},
           given={Graham},
           giveni={G\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=c129df5593fdaa7475548811bfbb227d}{%
           family={Erbriss},
           familyi={E\bibinitperiod},
           given={Edgar},
           giveni={E\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Grimble}%
      }
      \strng{namehash}{121b6dc164b5b619c81c670fbd823f12}
      \strng{fullhash}{121b6dc164b5b619c81c670fbd823f12}
      \strng{fullhashraw}{121b6dc164b5b619c81c670fbd823f12}
      \strng{bibnamehash}{121b6dc164b5b619c81c670fbd823f12}
      \strng{authorbibnamehash}{121b6dc164b5b619c81c670fbd823f12}
      \strng{authornamehash}{121b6dc164b5b619c81c670fbd823f12}
      \strng{authorfullhash}{121b6dc164b5b619c81c670fbd823f12}
      \strng{authorfullhashraw}{121b6dc164b5b619c81c670fbd823f12}
      \strng{editorbibnamehash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editornamehash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editorfullhash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editorfullhashraw}{c129df5593fdaa7475548811bfbb227d}
      \field{sortinit}{G}
      \field{sortinithash}{32d67eca0634bf53703493fb1090a2e8}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Graphs of the Continent}
      \strng{crossref}{cr_m}
      \field{eprintclass}{SOMECLASS}
      \field{eprinttype}{SomEPrFiX}
      \field{month}{1}
      \field{origyear}{1955}
      \field{title}{Great and Good Graphs}
      \field{year}{1974}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_CR2: &str = r#"    \entry{cr2}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=2d51a96bc0a6804995b3a9ff350c3384}{%
           family={Fumble},
           familyi={F\bibinitperiod},
           given={Frederick},
           giveni={F\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=c129df5593fdaa7475548811bfbb227d}{%
           family={Erbriss},
           familyi={E\bibinitperiod},
           given={Edgar},
           giveni={E\bibinitperiod}}}%
      }
      \list{institution}{1}{%
        {Institution}%
      }
      \list{publisher}{1}{%
        {Grimble}%
      }
      \strng{namehash}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{fullhash}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{fullhashraw}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{bibnamehash}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{authorbibnamehash}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{authornamehash}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{authorfullhash}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{authorfullhashraw}{2d51a96bc0a6804995b3a9ff350c3384}
      \strng{editorbibnamehash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editornamehash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editorfullhash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editorfullhashraw}{c129df5593fdaa7475548811bfbb227d}
      \field{sortinit}{F}
      \field{sortinithash}{2638baaa20439f1b5a8f80c6c08a13b4}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Graphs of the Continent}
      \strng{crossref}{cr_m}
      \field{origyear}{1943}
      \field{title}{Fabulous Fourier Forms}
      \field{year}{1974}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_CR_M: &str = r#"    \entry{cr_m}{book}{}{}
      \name{editor}{1}{}{%
        {{hash=c129df5593fdaa7475548811bfbb227d}{%
           family={Erbriss},
           familyi={E\bibinitperiod},
           given={Edgar},
           giveni={E\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Grimble}%
      }
      \strng{editorbibnamehash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editornamehash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editorfullhash}{c129df5593fdaa7475548811bfbb227d}
      \strng{editorfullhashraw}{c129df5593fdaa7475548811bfbb227d}
      \field{sortinit}{G}
      \field{sortinithash}{32d67eca0634bf53703493fb1090a2e8}
      \true{crossrefsource}
      \true{uniquetitle}
      \field{labeltitlesource}{title}
      \field{title}{Graphs of the Continent}
      \field{year}{1974}
    \endentry
"#;

const EXPECTED_CR3: &str = r#"    \entry{cr3}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=2baf676a220704f6914223aefccaaa88}{%
           family={Aptitude},
           familyi={A\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=a1f5c22413396d599ec766725b226735}{%
           family={Monkley},
           familyi={M\bibinitperiod},
           given={Mark},
           giveni={M\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Rancour}%
      }
      \strng{namehash}{2baf676a220704f6914223aefccaaa88}
      \strng{fullhash}{2baf676a220704f6914223aefccaaa88}
      \strng{fullhashraw}{2baf676a220704f6914223aefccaaa88}
      \strng{bibnamehash}{2baf676a220704f6914223aefccaaa88}
      \strng{authorbibnamehash}{2baf676a220704f6914223aefccaaa88}
      \strng{authornamehash}{2baf676a220704f6914223aefccaaa88}
      \strng{authorfullhash}{2baf676a220704f6914223aefccaaa88}
      \strng{authorfullhashraw}{2baf676a220704f6914223aefccaaa88}
      \strng{editorbibnamehash}{a1f5c22413396d599ec766725b226735}
      \strng{editornamehash}{a1f5c22413396d599ec766725b226735}
      \strng{editorfullhash}{a1f5c22413396d599ec766725b226735}
      \strng{editorfullhashraw}{a1f5c22413396d599ec766725b226735}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Beasts of the Burbling Burns}
      \strng{crossref}{crt}
      \field{eprinttype}{sometype}
      \field{origyear}{1934}
      \field{title}{Arrangements of All Articles}
      \field{year}{1996}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_CR4: &str = r#"    \entry{cr4}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=50ef7fd3a1be33bccc5de2768b013836}{%
           family={Mumble},
           familyi={M\bibinitperiod},
           given={Morris},
           giveni={M\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=6ea89bd4958743a20b70fe17647d6af5}{%
           family={Jermain},
           familyi={J\bibinitperiod},
           given={Jeremy},
           giveni={J\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Pillsbury}%
      }
      \strng{namehash}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{fullhash}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{fullhashraw}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{bibnamehash}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{authorbibnamehash}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{authornamehash}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{authorfullhash}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{authorfullhashraw}{50ef7fd3a1be33bccc5de2768b013836}
      \strng{editorbibnamehash}{6ea89bd4958743a20b70fe17647d6af5}
      \strng{editornamehash}{6ea89bd4958743a20b70fe17647d6af5}
      \strng{editorfullhash}{6ea89bd4958743a20b70fe17647d6af5}
      \strng{editorfullhashraw}{6ea89bd4958743a20b70fe17647d6af5}
      \field{sortinit}{M}
      \field{sortinithash}{4625c616857f13d17ce56f7d4f97d451}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Vanquished, Victor, Vandal}
      \field{origyear}{1911}
      \field{title}{Enterprising Entities}
      \field{year}{1945}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_CRT: &str = r#"    \entry{crt}{book}{}{}
      \name{editor}{1}{}{%
        {{hash=a1f5c22413396d599ec766725b226735}{%
           family={Monkley},
           familyi={M\bibinitperiod},
           given={Mark},
           giveni={M\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Rancour}%
      }
      \strng{editorbibnamehash}{a1f5c22413396d599ec766725b226735}
      \strng{editornamehash}{a1f5c22413396d599ec766725b226735}
      \strng{editorfullhash}{a1f5c22413396d599ec766725b226735}
      \strng{editorfullhashraw}{a1f5c22413396d599ec766725b226735}
      \field{sortinit}{B}
      \field{sortinithash}{d7095fff47cda75ca2589920aae98399}
      \true{uniquetitle}
      \field{labeltitlesource}{title}
      \field{title}{Beasts of the Burbling Burns}
      \field{year}{1996}
    \endentry
"#;

const EXPECTED_CR6: &str = r#"    \entry{cr6}{inproceedings}{}{}
      \name{author}{1}{}{%
        {{hash=8ab39ee68c55046dc1f05d657fcefed9}{%
           family={Author},
           familyi={A\bibinitperiod},
           given={Firstname},
           giveni={F\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=344a7f427fb765610ef96eb7bce95257}{%
           family={Editor},
           familyi={E\bibinitperiod}}}%
      }
      \list{location}{1}{%
        {Address}%
      }
      \strng{namehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{fullhash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{fullhashraw}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{bibnamehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authorbibnamehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authornamehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authorfullhash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authorfullhashraw}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{editorbibnamehash}{344a7f427fb765610ef96eb7bce95257}
      \strng{editornamehash}{344a7f427fb765610ef96eb7bce95257}
      \strng{editorfullhash}{344a7f427fb765610ef96eb7bce95257}
      \strng{editorfullhashraw}{344a7f427fb765610ef96eb7bce95257}
      \field{extraname}{2}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Manual booktitle}
      \field{eventday}{21}
      \field{eventendday}{24}
      \field{eventendmonth}{8}
      \field{eventendyear}{2009}
      \field{eventmonth}{8}
      \field{eventtitle}{Title of the event}
      \field{eventyear}{2009}
      \field{title}{Title of inproceeding}
      \field{venue}{Location of event}
      \field{year}{2009}
      \field{eventenddateera}{ce}
      \field{eventdateera}{ce}
      \field{pages}{123\bibrangedash}
      \range{pages}{-1}
    \endentry
"#;

const EXPECTED_CR7: &str = r#"    \entry{cr7}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=8ab39ee68c55046dc1f05d657fcefed9}{%
           family={Author},
           familyi={A\bibinitperiod},
           given={Firstname},
           giveni={F\bibinitperiod}}}%
      }
      \name{bookauthor}{1}{}{%
        {{hash=91a1dd4aeed3c4ec29ca74c4e778be5f}{%
           family={Bookauthor},
           familyi={B\bibinitperiod},
           given={Brian},
           giveni={B\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Publisher of proceeding}%
      }
      \strng{namehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{fullhash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{fullhashraw}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{bibnamehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authorbibnamehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authornamehash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authorfullhash}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{authorfullhashraw}{8ab39ee68c55046dc1f05d657fcefed9}
      \strng{bookauthorbibnamehash}{91a1dd4aeed3c4ec29ca74c4e778be5f}
      \strng{bookauthornamehash}{91a1dd4aeed3c4ec29ca74c4e778be5f}
      \strng{bookauthorfullhash}{91a1dd4aeed3c4ec29ca74c4e778be5f}
      \strng{bookauthorfullhashraw}{91a1dd4aeed3c4ec29ca74c4e778be5f}
      \field{extraname}{1}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booksubtitle}{Book Subtitle}
      \field{booktitle}{Book Title}
      \field{booktitleaddon}{Book Titleaddon}
      \field{title}{Title of Book bit}
      \field{year}{2010}
      \field{pages}{123\bibrangedash 126}
      \range{pages}{4}
      \verb{verbb}
      \verb String
      \endverb
    \endentry
"#;

const EXPECTED_CR8: &str = r#"    \entry{cr8}{incollection}{}{}
      \name{author}{1}{}{%
        {{hash=3d449e56eb3ca1ae80dc99a18d689795}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Firstname},
           giveni={F\bibinitperiod}}}%
      }
      \strng{namehash}{3d449e56eb3ca1ae80dc99a18d689795}
      \strng{fullhash}{3d449e56eb3ca1ae80dc99a18d689795}
      \strng{fullhashraw}{3d449e56eb3ca1ae80dc99a18d689795}
      \strng{bibnamehash}{3d449e56eb3ca1ae80dc99a18d689795}
      \strng{authorbibnamehash}{3d449e56eb3ca1ae80dc99a18d689795}
      \strng{authornamehash}{3d449e56eb3ca1ae80dc99a18d689795}
      \strng{authorfullhash}{3d449e56eb3ca1ae80dc99a18d689795}
      \strng{authorfullhashraw}{3d449e56eb3ca1ae80dc99a18d689795}
      \field{extraname}{4}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{booktitle}{Book Title}
      \field{title}{Title of Collection bit}
      \field{year}{2010}
      \field{pages}{1\bibrangedash 12}
      \range{pages}{12}
    \endentry
"#;

const EXPECTED_XR1: &str = r#"    \entry{xr1}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=e0ecc4fc668ee499d1afba44e1ac064d}{%
           family={Zentrum},
           familyi={Z\bibinitperiod},
           given={Zoe},
           giveni={Z\bibinitperiod}}}%
      }
      \strng{namehash}{e0ecc4fc668ee499d1afba44e1ac064d}
      \strng{fullhash}{e0ecc4fc668ee499d1afba44e1ac064d}
      \strng{fullhashraw}{e0ecc4fc668ee499d1afba44e1ac064d}
      \strng{bibnamehash}{e0ecc4fc668ee499d1afba44e1ac064d}
      \strng{authorbibnamehash}{e0ecc4fc668ee499d1afba44e1ac064d}
      \strng{authornamehash}{e0ecc4fc668ee499d1afba44e1ac064d}
      \strng{authorfullhash}{e0ecc4fc668ee499d1afba44e1ac064d}
      \strng{authorfullhashraw}{e0ecc4fc668ee499d1afba44e1ac064d}
      \field{sortinit}{Z}
      \field{sortinithash}{96892c0b0a36bb8557c40c49813d48b3}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{origyear}{1921}
      \field{title}{Moods Mildly Modified}
      \strng{xref}{xrm}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_XR2: &str = r#"    \entry{xr2}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=6afa09374ecfd6b394ce714d2d9709c7}{%
           family={Instant},
           familyi={I\bibinitperiod},
           given={Ian},
           giveni={I\bibinitperiod}}}%
      }
      \strng{namehash}{6afa09374ecfd6b394ce714d2d9709c7}
      \strng{fullhash}{6afa09374ecfd6b394ce714d2d9709c7}
      \strng{fullhashraw}{6afa09374ecfd6b394ce714d2d9709c7}
      \strng{bibnamehash}{6afa09374ecfd6b394ce714d2d9709c7}
      \strng{authorbibnamehash}{6afa09374ecfd6b394ce714d2d9709c7}
      \strng{authornamehash}{6afa09374ecfd6b394ce714d2d9709c7}
      \strng{authorfullhash}{6afa09374ecfd6b394ce714d2d9709c7}
      \strng{authorfullhashraw}{6afa09374ecfd6b394ce714d2d9709c7}
      \field{sortinit}{I}
      \field{sortinithash}{8d291c51ee89b6cd86bf5379f0b151d8}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{origyear}{1926}
      \field{title}{Migraines Multiplying Madly}
      \strng{xref}{xrm}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_XRM: &str = r#"    \entry{xrm}{book}{}{}
      \name{editor}{1}{}{%
        {{hash=809950f9b59ae207092b909a19dcb27b}{%
           family={Prendergast},
           familyi={P\bibinitperiod},
           given={Peter},
           giveni={P\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Mainstream}%
      }
      \strng{editorbibnamehash}{809950f9b59ae207092b909a19dcb27b}
      \strng{editornamehash}{809950f9b59ae207092b909a19dcb27b}
      \strng{editorfullhash}{809950f9b59ae207092b909a19dcb27b}
      \strng{editorfullhashraw}{809950f9b59ae207092b909a19dcb27b}
      \field{sortinit}{C}
      \field{sortinithash}{4d103a86280481745c9c897c925753c0}
      \true{xrefsource}
      \true{uniquetitle}
      \field{labeltitlesource}{title}
      \field{title}{Calligraphy, Calisthenics, Culture}
      \field{year}{1970}
    \endentry
"#;

const EXPECTED_XR3: &str = r#"    \entry{xr3}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=9788055665b9bb4b37c776c3f6b74f16}{%
           family={Normal},
           familyi={N\bibinitperiod},
           given={Norman},
           giveni={N\bibinitperiod}}}%
      }
      \strng{namehash}{9788055665b9bb4b37c776c3f6b74f16}
      \strng{fullhash}{9788055665b9bb4b37c776c3f6b74f16}
      \strng{fullhashraw}{9788055665b9bb4b37c776c3f6b74f16}
      \strng{bibnamehash}{9788055665b9bb4b37c776c3f6b74f16}
      \strng{authorbibnamehash}{9788055665b9bb4b37c776c3f6b74f16}
      \strng{authornamehash}{9788055665b9bb4b37c776c3f6b74f16}
      \strng{authorfullhash}{9788055665b9bb4b37c776c3f6b74f16}
      \strng{authorfullhashraw}{9788055665b9bb4b37c776c3f6b74f16}
      \field{sortinit}{N}
      \field{sortinithash}{22369a73d5f88983a108b63f07f37084}
      \true{singletitle}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{origyear}{1923}
      \field{title}{Russian Regalia Revisited}
      \strng{xref}{xrt}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_XRT: &str = r#"    \entry{xrt}{book}{}{}
      \name{editor}{1}{}{%
        {{hash=bf7d6b02f3e073913e5bfe5059508dd5}{%
           family={Lunders},
           familyi={L\bibinitperiod},
           given={Lucy},
           giveni={L\bibinitperiod}}}%
      }
      \list{publisher}{1}{%
        {Middling}%
      }
      \strng{editorbibnamehash}{bf7d6b02f3e073913e5bfe5059508dd5}
      \strng{editornamehash}{bf7d6b02f3e073913e5bfe5059508dd5}
      \strng{editorfullhash}{bf7d6b02f3e073913e5bfe5059508dd5}
      \strng{editorfullhashraw}{bf7d6b02f3e073913e5bfe5059508dd5}
      \field{sortinit}{K}
      \field{sortinithash}{c02bf6bff1c488450c352b40f5d853ab}
      \true{uniquetitle}
      \field{labeltitlesource}{title}
      \field{title}{Kings, Cork and Calculation}
      \field{year}{1977}
    \endentry
"#;

const EXPECTED_XR4: &str = r#"    \entry{xr4}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=7804ffef086c0c4686c235807f5cb502}{%
           family={Mistrel},
           familyi={M\bibinitperiod},
           given={Megan},
           giveni={M\bibinitperiod}}}%
      }
      \strng{namehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{fullhash}{7804ffef086c0c4686c235807f5cb502}
      \strng{fullhashraw}{7804ffef086c0c4686c235807f5cb502}
      \strng{bibnamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorbibnamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authornamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorfullhash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorfullhashraw}{7804ffef086c0c4686c235807f5cb502}
      \field{extraname}{1}
      \field{sortinit}{M}
      \field{sortinithash}{4625c616857f13d17ce56f7d4f97d451}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{origyear}{1933}
      \field{title}{Lumbering Lunatics}
      \strng{xref}{xrn}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_MXR: &str = r#"    \entry{mxr}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=7804ffef086c0c4686c235807f5cb502}{%
           family={Mistrel},
           familyi={M\bibinitperiod},
           given={Megan},
           giveni={M\bibinitperiod}}}%
      }
      \strng{namehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{fullhash}{7804ffef086c0c4686c235807f5cb502}
      \strng{fullhashraw}{7804ffef086c0c4686c235807f5cb502}
      \strng{bibnamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorbibnamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authornamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorfullhash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorfullhashraw}{7804ffef086c0c4686c235807f5cb502}
      \field{extraname}{2}
      \field{sortinit}{M}
      \field{sortinithash}{4625c616857f13d17ce56f7d4f97d451}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{origyear}{1933}
      \field{title}{Lumbering Lunatics}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_MCR: &str = r#"    \entry{mcr}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=7804ffef086c0c4686c235807f5cb502}{%
           family={Mistrel},
           familyi={M\bibinitperiod},
           given={Megan},
           giveni={M\bibinitperiod}}}%
      }
      \strng{namehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{fullhash}{7804ffef086c0c4686c235807f5cb502}
      \strng{fullhashraw}{7804ffef086c0c4686c235807f5cb502}
      \strng{bibnamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorbibnamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authornamehash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorfullhash}{7804ffef086c0c4686c235807f5cb502}
      \strng{authorfullhashraw}{7804ffef086c0c4686c235807f5cb502}
      \field{extraname}{3}
      \field{sortinit}{M}
      \field{sortinithash}{4625c616857f13d17ce56f7d4f97d451}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{origyear}{1933}
      \field{title}{Lumbering Lunatics}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_CCR1: &str = r#"    \entry{ccr2}{book}{}{}
      \name{author}{1}{}{%
        {{hash=6268941b408d3263bddb208a54899ea9}{%
           family={Various},
           familyi={V\bibinitperiod},
           given={Vince},
           giveni={V\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=cfee758a1c82df2e26af1985e061bb0a}{%
           family={Editor},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod}}}%
      }
      \strng{namehash}{6268941b408d3263bddb208a54899ea9}
      \strng{fullhash}{6268941b408d3263bddb208a54899ea9}
      \strng{fullhashraw}{6268941b408d3263bddb208a54899ea9}
      \strng{bibnamehash}{6268941b408d3263bddb208a54899ea9}
      \strng{authorbibnamehash}{6268941b408d3263bddb208a54899ea9}
      \strng{authornamehash}{6268941b408d3263bddb208a54899ea9}
      \strng{authorfullhash}{6268941b408d3263bddb208a54899ea9}
      \strng{authorfullhashraw}{6268941b408d3263bddb208a54899ea9}
      \strng{editorbibnamehash}{cfee758a1c82df2e26af1985e061bb0a}
      \strng{editornamehash}{cfee758a1c82df2e26af1985e061bb0a}
      \strng{editorfullhash}{cfee758a1c82df2e26af1985e061bb0a}
      \strng{editorfullhashraw}{cfee758a1c82df2e26af1985e061bb0a}
      \field{extraname}{1}
      \field{sortinit}{V}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \true{uniquetitle}
      \true{uniquework}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \strng{crossref}{ccr1}
      \field{title}{Misc etc.}
      \field{year}{1923}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_CCR2: &str = r#"    \entry{ccr3}{inbook}{}{}
      \name{bookauthor}{1}{}{%
        {{hash=6268941b408d3263bddb208a54899ea9}{%
           family={Various},
           familyi={V\bibinitperiod},
           given={Vince},
           giveni={V\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=cfee758a1c82df2e26af1985e061bb0a}{%
           family={Editor},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod}}}%
      }
      \strng{bookauthorbibnamehash}{6268941b408d3263bddb208a54899ea9}
      \strng{bookauthornamehash}{6268941b408d3263bddb208a54899ea9}
      \strng{bookauthorfullhash}{6268941b408d3263bddb208a54899ea9}
      \strng{bookauthorfullhashraw}{6268941b408d3263bddb208a54899ea9}
      \strng{editorbibnamehash}{cfee758a1c82df2e26af1985e061bb0a}
      \strng{editornamehash}{cfee758a1c82df2e26af1985e061bb0a}
      \strng{editorfullhash}{cfee758a1c82df2e26af1985e061bb0a}
      \strng{editorfullhashraw}{cfee758a1c82df2e26af1985e061bb0a}
      \field{sortinit}{P}
      \field{sortinithash}{ff3bcf24f47321b42cb156c2cc8a8422}
      \true{uniquetitle}
      \field{labeltitlesource}{title}
      \field{booktitle}{Misc etc.}
      \strng{crossref}{ccr2}
      \field{title}{Perhaps, Perchance, Possibilities?}
      \field{year}{1911}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_S1: &str = r#"    \entry{s1}{inbook}{}{}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \true{uniquetitle}
      \field{labeltitlesource}{title}
      \strng{crossref}{s2}
      \field{title}{Subtitle}
    \endentry
"#;

const EXPECTED_XC2: &str = r#"    \entry{xc2}{inbook}{}{}
      \name{author}{1}{}{%
        {{hash=1a0f7d518cccdad859a74412ef956474}{%
           family={Crust},
           familyi={C\\bibinitperiod},
           given={Xavier},
           giveni={X\\bibinitperiod}}}%
      }
      \name{bookauthor}{1}{}{%
        {{hash=1a0f7d518cccdad859a74412ef956474}{%
           family={Crust},
           familyi={C\\bibinitperiod},
           given={Xavier},
           giveni={X\\bibinitperiod}}}%
      }
      \strng{namehash}{1a0f7d518cccdad859a74412ef956474}
      \strng{fullhash}{1a0f7d518cccdad859a74412ef956474}
      \strng{fullhashraw}{1a0f7d518cccdad859a74412ef956474}
      \strng{bibnamehash}{1a0f7d518cccdad859a74412ef956474}
      \strng{authorbibnamehash}{1a0f7d518cccdad859a74412ef956474}
      \strng{authornamehash}{1a0f7d518cccdad859a74412ef956474}
      \strng{authorfullhash}{1a0f7d518cccdad859a74412ef956474}
      \strng{authorfullhashraw}{1a0f7d518cccdad859a74412ef956474}
      \strng{bookauthorbibnamehash}{1a0f7d518cccdad859a74412ef956474}
      \strng{bookauthornamehash}{1a0f7d518cccdad859a74412ef956474}
      \strng{bookauthorfullhash}{1a0f7d518cccdad859a74412ef956474}
      \strng{bookauthorfullhashraw}{1a0f7d518cccdad859a74412ef956474}
      \field{extraname}{2}
      \field{sortinit}{C}
      \field{sortinithash}{4d103a86280481745c9c897c925753c0}
      \true{xrefsource}
      \field{labelnamesource}{author}
      \field{booktitle}{Title}
    \endentry
"#;

const EXPECTED_B1: &str = r#"    \entry{b1}{inbook}{}{}
      \field{sortinit}{2}
      \field{sortinithash}{8b555b3791beccb63322c22f3320aa9a}
      \strng{crossref}{b2}
      \field{day}{3}
      \field{month}{3}
      \field{origmonth}{3}
      \field{origyear}{2004}
      \field{year}{2004}
      \field{dateera}{ce}
      \field{origdateera}{ce}
    \endentry
"#;

const EXPECTED_SUP1: &str = r#"    \entry{sup1}{mvbook}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{3}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \true{crossrefsource}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{title}{Title1}
    \endentry
"#;

const EXPECTED_SUP2: &str = r#"    \entry{sup2}{book}{}{}
      \name{author}{1}{}{%
        {{hash=556c8dba145b472e6a8598d506f7cbe2}{%
           family={Smith},
           familyi={S\\bibinitperiod},
           given={Alan},
           giveni={A\\bibinitperiod}}}%
      }
      \strng{namehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{fullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{bibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorbibnamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authornamehash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhash}{556c8dba145b472e6a8598d506f7cbe2}
      \strng{authorfullhashraw}{556c8dba145b472e6a8598d506f7cbe2}
      \field{extraname}{1}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \true{singletitle}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \strng{crossref}{sup1}
      \field{note}{Book sup2}
      \field{title}{Title1}
    \endentry
"#;

fn has_citekey(result: &bib_engine::BibResult, section: u32, key: &str) -> bool {
    result
        .document()
        .section(bib_engine::SectionId::new(section))
        .is_some_and(|section| {
            section
                .lists()
                .any(|list| list.entries().any(|entry| entry.as_str() == key))
        })
}

fn rendered_errors(result: &bib_engine::BibResult) -> String {
    result
        .diagnostics()
        .filter(|diagnostic| diagnostic.severity() == bib_engine::BibSeverity::Error)
        .map(|diagnostic| format!("ERROR - {}", diagnostic.message()))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_001_crossref_test_1() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr1"))
            .as_deref(),
        Some(EXPECTED_CR1)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_002_crossref_test_2() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr2"))
            .as_deref(),
        Some(EXPECTED_CR2)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_003_crossref_test_3() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr_m"))
            .as_deref(),
        Some(EXPECTED_CR_M)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_004_crossref_test_4() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr3"))
            .as_deref(),
        Some(EXPECTED_CR3)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_005_crossref_test_5() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "crt"))
            .as_deref(),
        Some(EXPECTED_CRT)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_006_crossref_test_6() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr4"))
            .as_deref(),
        Some(EXPECTED_CR4)
    );
}

#[test]
fn assertion_007_crossref_test_7() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 0, "crn")),
        false
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_008_crossref_test_inheritance_8() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr6"))
            .as_deref(),
        Some(EXPECTED_CR6)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_009_crossref_test_inheritance_9() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr7"))
            .as_deref(),
        Some(EXPECTED_CR7)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_010_crossref_test_inheritance_10() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "cr8"))
            .as_deref(),
        Some(EXPECTED_CR8)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_011_xref_test_1() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xr1"))
            .as_deref(),
        Some(EXPECTED_XR1)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_012_xref_test_2() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xr2"))
            .as_deref(),
        Some(EXPECTED_XR2)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_013_xref_test_3() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xrm"))
            .as_deref(),
        Some(EXPECTED_XRM)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_014_xref_test_4() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xr3"))
            .as_deref(),
        Some(EXPECTED_XR3)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_015_xref_test_5() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xrt"))
            .as_deref(),
        Some(EXPECTED_XRT)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_016_xref_test_6() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xr4"))
            .as_deref(),
        Some(EXPECTED_XR4)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_017_xref_test_7() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 0, "xrn")),
        true
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_018_missing_xref_test() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "mxr"))
            .as_deref(),
        Some(EXPECTED_MXR)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_019_missing_crossef_test() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "mcr"))
            .as_deref(),
        Some(EXPECTED_MCR)
    );
}

#[test]
fn assertion_020_mincrossrefs_reset_between_sections() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 1, "crn")),
        false
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_021_cascading_crossref_test_1() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "ccr2"))
            .as_deref(),
        Some(EXPECTED_CCR1)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_022_cascading_crossref_test_2() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "ccr3"))
            .as_deref(),
        Some(EXPECTED_CCR2)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_023_cyclic_crossref_error_check() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result.as_ref().ok().map(rendered_errors).as_deref(),
        Some(
            "ERROR - Circular inheritance between 'circ1'<->'circ2'\nERROR - Circular inheritance between 'circ3'<->'circ1'"
        )
    );
}

#[test]
fn assertion_024_recursive_crossref_test_1() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 0, "r1")),
        true
    );
}

#[test]
fn assertion_025_recursive_crossref_test_2() {
    let result = try_run_fixture("crossrefs");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "r1"))
            .is_some()
    );
}

#[test]
fn assertion_026_recursive_crossref_test_3() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 0, "r2")),
        false
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_027_recursive_crossref_test_4() {
    let result = try_run_fixture("crossrefs");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "r2"))
            .is_some()
    );
}

#[test]
fn assertion_028_recursive_crossref_test_5() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 0, "r3")),
        false
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_029_recursive_crossref_test_6() {
    let result = try_run_fixture("crossrefs");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "r3"))
            .is_some()
    );
}

#[test]
fn assertion_030_recursive_crossref_test_7() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 0, "r4")),
        false
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_031_recursive_crossref_test_8() {
    let result = try_run_fixture("crossrefs");
    assert!(
        result
            .as_ref()
            .ok()
            .and_then(|result| entry(result, 0, "r4"))
            .is_some()
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_032_per_entry_noinherit() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "s1"))
            .as_deref(),
        Some(EXPECTED_S1)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_033_cascading_xref_crossref() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xc2"))
            .as_deref(),
        Some(EXPECTED_XC2)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_034_blocking_bad_date_inheritance() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "b1"))
            .as_deref(),
        Some(EXPECTED_B1)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_035_suppressing_singletitle_tracking_1() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "sup1"))
            .as_deref(),
        Some(EXPECTED_SUP1)
    );
}

#[test]
#[ignore = "xfail: Biber crossref/xref inheritance is not implemented by bib-engine"]
fn assertion_036_suppressing_singletitle_tracking_2() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "sup2"))
            .as_deref(),
        Some(EXPECTED_SUP2)
    );
}

#[test]
fn assertion_037_mincrossref_via_alias() {
    let result = try_run_fixture("crossrefs");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .is_some_and(|result| has_citekey(result, 0, "al2")),
        false
    );
}
