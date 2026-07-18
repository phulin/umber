//! Native translations of upstream `t/xdata.t` at commit 74252e6.

use super::maps::{output_entry, try_run_fixture};

const EXPECTED_XD1: &str = r#"    \entry{xd1}{book}{}{}
      \name{author}{1}{}{%
        {{hash=51db4bfd331cba22959ce2d224c517cd}{%
           family={Ellington},
           familyi={E\bibinitperiod},
           given={Edward},
           giveni={E\bibinitperiod}}}%
      }
      \list{location}{2}{%
        {New York}%
        {London}%
      }
      \list{publisher}{1}{%
        {Macmillan}%
      }
      \strng{namehash}{51db4bfd331cba22959ce2d224c517cd}
      \strng{fullhash}{51db4bfd331cba22959ce2d224c517cd}
      \strng{fullhashraw}{51db4bfd331cba22959ce2d224c517cd}
      \strng{bibnamehash}{51db4bfd331cba22959ce2d224c517cd}
      \strng{authorbibnamehash}{51db4bfd331cba22959ce2d224c517cd}
      \strng{authornamehash}{51db4bfd331cba22959ce2d224c517cd}
      \strng{authorfullhash}{51db4bfd331cba22959ce2d224c517cd}
      \strng{authorfullhashraw}{51db4bfd331cba22959ce2d224c517cd}
      \field{extraname}{2}
      \field{sortinit}{E}
      \field{sortinithash}{8da8a182d344d5b9047633dfc0cc9131}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{note}{A Note}
      \field{year}{2007}
      \field{dateera}{ce}
      \warn{\item book entry 'xd1' references XDATA entry 'missingxd' which does not exist, not resolving (section 0)}
    \endentry
"#;

const EXPECTED_XD2: &str = r#"    \entry{xd2}{book}{}{}
      \name{author}{1}{}{%
        {{hash=68539e0ce4922cc4957c6cabf35e6fc8}{%
           family={Pillington},
           familyi={P\bibinitperiod},
           given={Peter},
           giveni={P\bibinitperiod}}}%
      }
      \list{location}{2}{%
        {New York}%
        {London}%
      }
      \list{publisher}{1}{%
        {Routledge}%
      }
      \strng{namehash}{68539e0ce4922cc4957c6cabf35e6fc8}
      \strng{fullhash}{68539e0ce4922cc4957c6cabf35e6fc8}
      \strng{fullhashraw}{68539e0ce4922cc4957c6cabf35e6fc8}
      \strng{bibnamehash}{68539e0ce4922cc4957c6cabf35e6fc8}
      \strng{authorbibnamehash}{68539e0ce4922cc4957c6cabf35e6fc8}
      \strng{authornamehash}{68539e0ce4922cc4957c6cabf35e6fc8}
      \strng{authorfullhash}{68539e0ce4922cc4957c6cabf35e6fc8}
      \strng{authorfullhashraw}{68539e0ce4922cc4957c6cabf35e6fc8}
      \field{sortinit}{P}
      \field{sortinithash}{ff3bcf24f47321b42cb156c2cc8a8422}
      \field{extradatescope}{labelyear}
      \field{labeldatesource}{}
      \field{labelnamesource}{author}
      \field{abstract}{An abstract}
      \field{addendum}{Москва}
      \field{note}{A Note}
      \field{venue}{venue}
      \field{year}{2003}
      \field{dateera}{ce}
    \endentry
"#;

const EXPECTED_GXD1: &str = r#"    \entry{gxd1}{book}{}{}
      \name{author}{2}{}{%
        {{hash=6b3653417f9aa97391c37cff5dfda7fa}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Simon},
           giveni={S\bibinitperiod}}}%
        {{hash=350a836ae63897de6d88baf1d62dc9f2}{%
           family={Bloom},
           familyi={B\bibinitperiod},
           given={Brian},
           giveni={B\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=6238b302317c6baeba56035f2c4998c9}{%
           family={Frill},
           familyi={F\bibinitperiod},
           given={Frank},
           giveni={F\bibinitperiod}}}%
      }
      \name{namea}{1}{}{%
        {{hash=d41d8cd98f00b204e9800998ecf8427e}{%
}}%
      }
      \name{translator}{1}{}{%
        {{hash=d41d8cd98f00b204e9800998ecf8427e}{%
}}%
      }
      \list{lista}{1}{%
        {xdata=gxd3-location-5}%
      }
      \list{location}{2}{%
        {A}%
        {B}%
      }
      \list{organization}{1}{%
        {xdata=gxd2-author-3}%
      }
      \list{publisher}{1}{%
        {xdata=gxd2}%
      }
      \strng{namehash}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{fullhash}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{fullhashraw}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{bibnamehash}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{authorbibnamehash}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{authornamehash}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{authorfullhash}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{authorfullhashraw}{167d3a67f6ee19fe4d131fc34dcd9ede}
      \strng{editorbibnamehash}{6238b302317c6baeba56035f2c4998c9}
      \strng{editornamehash}{6238b302317c6baeba56035f2c4998c9}
      \strng{editorfullhash}{6238b302317c6baeba56035f2c4998c9}
      \strng{editorfullhashraw}{6238b302317c6baeba56035f2c4998c9}
      \strng{nameabibnamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{nameanamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{nameafullhash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{nameafullhashraw}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorbibnamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatornamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorfullhash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorfullhashraw}{d41d8cd98f00b204e9800998ecf8427e}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{addendum}{xdata=missing}
      \field{note}{xdata=gxd2-note}
      \field{title}{Some title}
      \warn{\item book entry 'gxd1' has XDATA reference from field 'publisher' that contains no source field (section 0)}
      \warn{\\item book entry 'gxd1' has XDATA reference from field 'addendum' that contains no source field (section 0)}
      \warn{\item Field 'note' in book entry 'gxd1' references XDATA field 'note' in entry 'gxd2' and this field does not exist, not resolving (section 0)}
      \warn{\item Field 'translator' in book entry 'gxd1' references field 'author' position 3 in entry 'gxd2' and this position does not exist, not resolving (section 0)}
      \warn{\item Field 'lista' in book entry 'gxd1' references field 'location' position 5 in entry 'gxd3' and this position does not exist, not resolving (section 0)}
      \warn{\item Field 'organization' in book entry 'gxd1' which xdata references field 'author' in entry 'gxd2' are not the same types, not resolving (section 0)}
      \warn{\item book entry 'gxd1' references XDATA entry 'lxd1' which is not an XDATA entry, not resolving (section 0)}
    \endentry
"#;

const EXPECTED_GXD1G: &str = r#"    \entry{gxd1g}{book}{}{}
      \name{author}{3}{}{%
        {{hash=6b3653417f9aa97391c37cff5dfda7fa}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Simon},
           giveni={S\bibinitperiod}}}%
        {{hash=350a836ae63897de6d88baf1d62dc9f2}{%
           family={Bloom},
           familyi={B\bibinitperiod},
           given={Brian},
           giveni={B\bibinitperiod}}}%
        {{hash=7370e41a0804af6d5598ecf557c59841}{%
           family={Anderson},
           familyi={A\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=6238b302317c6baeba56035f2c4998c9}{%
           family={Frill},
           familyi={F\bibinitperiod},
           given={Frank},
           giveni={F\bibinitperiod}}}%
      }
      \name{namea}{1}{}{%
        {{hash=d41d8cd98f00b204e9800998ecf8427e}{%
}}%
      }
      \name{translator}{1}{}{%
        {{hash=d41d8cd98f00b204e9800998ecf8427e}{%
}}%
      }
      \list{lista}{1}{%
        {xdata=gxd3-location-5}%
      }
      \list{location}{3}{%
        {A}%
        {C}%
        {B}%
      }
      \list{organization}{1}{%
        {xdata=gxd2-author-3}%
      }
      \list{publisher}{1}{%
        {xdata=gxd2}%
      }
      \strng{namehash}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{fullhash}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{fullhashraw}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{bibnamehash}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{authorbibnamehash}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{authornamehash}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{authorfullhash}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{authorfullhashraw}{9fd3d5e0bec66ae3baacf58cf747485a}
      \strng{editorbibnamehash}{6238b302317c6baeba56035f2c4998c9}
      \strng{editornamehash}{6238b302317c6baeba56035f2c4998c9}
      \strng{editorfullhash}{6238b302317c6baeba56035f2c4998c9}
      \strng{editorfullhashraw}{6238b302317c6baeba56035f2c4998c9}
      \strng{nameabibnamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{nameanamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{nameafullhash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{nameafullhashraw}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorbibnamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatornamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorfullhash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorfullhashraw}{d41d8cd98f00b204e9800998ecf8427e}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{addendum}{xdata=missing}
      \field{note}{xdata=gxd2-note}
      \field{title}{Some title}
      \warn{\item book entry 'gxd1g' has XDATA reference from field 'publisher' that contains no source field (section 0)}
      \warn{\item book entry 'gxd1g' has XDATA reference from field 'addendum' that contains no source field (section 0)}
      \warn{\item Field 'note' in book entry 'gxd1g' references XDATA field 'note' in entry 'gxd2' and this field does not exist, not resolving (section 0)}
      \warn{\item Field 'translator' in book entry 'gxd1g' references field 'author' position 3 in entry 'gxd2' and this position does not exist, not resolving (section 0)}
      \warn{\item Field 'lista' in book entry 'gxd1g' references field 'location' position 5 in entry 'gxd3' and this position does not exist, not resolving (section 0)}
      \warn{\item Field 'organization' in book entry 'gxd1g' which xdata references field 'author' in entry 'gxd2' are not the same types, not resolving (section 0)}
      \warn{\item book entry 'gxd1g' references XDATA entry 'lxd1' which is not an XDATA entry, not resolving (section 0)}
    \endentry
"#;

const EXPECTED_BLTXGXD1: &str = r#"    \entry{bltxgxd1}{book}{}{}
      \name{author}{2}{}{%
        {{hash=ecc4a87e596c582a09b19d4ab187d8c2}{%
           family={Brian},
           familyi={B\bibinitperiod},
           given={Bell},
           giveni={B\bibinitperiod}}}%
        {{hash=aec59e82011f45e1e719b313e70abfdc}{%
           family={Clive},
           familyi={C\bibinitperiod},
           given={Clue},
           giveni={C\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=c8eb0270ad4e434f36dca28e219e81a8}{%
           family={Lee},
           familyi={L\bibinitperiod},
           given={Lay},
           giveni={L\bibinitperiod}}}%
      }
      \name{translator}{1}{}{%
        {{hash=d41d8cd98f00b204e9800998ecf8427e}{%
}}%
      }
      \list{lista}{1}{%
        {xdata=bltxgxd3-location-5}%
      }
      \list{location}{2}{%
        {A}%
        {B}%
      }
      \list{organization}{1}{%
        {xdata=bltxgxd2-author-3}%
      }
      \list{publisher}{1}{%
        {xdata=bltxgxd2}%
      }
      \strng{namehash}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{fullhash}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{fullhashraw}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{bibnamehash}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{authorbibnamehash}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{authornamehash}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{authorfullhash}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{authorfullhashraw}{f3cbd0df6512c5a3653f60e9e9849c69}
      \strng{editorbibnamehash}{c8eb0270ad4e434f36dca28e219e81a8}
      \strng{editornamehash}{c8eb0270ad4e434f36dca28e219e81a8}
      \strng{editorfullhash}{c8eb0270ad4e434f36dca28e219e81a8}
      \strng{editorfullhashraw}{c8eb0270ad4e434f36dca28e219e81a8}
      \strng{translatorbibnamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatornamehash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorfullhash}{d41d8cd98f00b204e9800998ecf8427e}
      \strng{translatorfullhashraw}{d41d8cd98f00b204e9800998ecf8427e}
      \field{sortinit}{B}
      \field{sortinithash}{d7095fff47cda75ca2589920aae98399}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{addendum}{xdata=missing}
      \field{note}{xdata=bltxgxd2-note}
      \field{title}{Some title}
      \warn{\item book entry 'bltxgxd1' has XDATA reference from field 'publisher' that contains no source field (section 0)}
      \warn{\item book entry 'bltxgxd1' has XDATA reference from field 'addendum' that contains no source field (section 0)}
      \warn{\item Field 'translator' in book entry 'bltxgxd1' references field 'author' position 3 in entry 'bltxgxd2' and this position does not exist, not resolving (section 0)}
      \warn{\item Field 'lista' in book entry 'bltxgxd1' references field 'location' position 5 in entry 'bltxgxd3' and this position does not exist, not resolving (section 0)}
      \warn{\item Field 'organization' in book entry 'bltxgxd1' which xdata references field 'author' in entry 'bltxgxd2' are not the same types, not resolving (section 0)}
      \warn{\item Field 'note' in book entry 'bltxgxd1' references XDATA field 'note' in entry 'bltxgxd2' and this field does not exist, not resolving (section 0)}
    \endentry
"#;

const EXPECTED_XDANN1: &str = r#"    \entry{xdann1}{book}{}{}
      \name{author}{4}{}{%
        {{hash=9c855075c7ab53ad38ec38086eda2029}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod}}}%
        {{hash=0c6731af5e4274be0b0ceef16eccb8f6}{%
           family={Bee},
           familyi={B\bibinitperiod},
           given={May},
           giveni={M\bibinitperiod}}}%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=ccc542396e5b42506590dc7132859c8c}{%
           family={Blogs},
           familyi={B\bibinitperiod},
           given={Bill},
           giveni={B\bibinitperiod}}}%
      }
      \name{editor}{5}{}{%
        {{hash=93f025f0446f3db59decfaf17a19dbbe}{%
           family={Little},
           familyi={L\bibinitperiod},
           given={Raymond},
           giveni={R\bibinitperiod}}}%
        {{hash=bd051a2f7a5f377e3a62581b0e0f8577}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod}}}%
        {{hash=d6cfb2b8c4b3f9440ec4642438129367}{%
           family={Doe},
           familyi={D\bibinitperiod},
           given={Jane},
           giveni={J\bibinitperiod}}}%
        {{hash=0c6731af5e4274be0b0ceef16eccb8f6}{%
           family={Bee},
           familyi={B\bibinitperiod},
           given={May},
           giveni={M\bibinitperiod}}}%
        {{hash=ead97b429847e5d377495ef9e13acb27}{%
           family={Roth},
           familyi={R\bibinitperiod},
           given={Gerald},
           giveni={G\bibinitperiod}}}%
      }
      \list{institution}{3}{%
        {inst1}%
        {inst2}%
        {inst3}%
      }
      \list{location}{3}{%
        {loca}%
        {xloc2}%
        {xloc2}%
      }
      \list{publisher}{1}{%
        {MacMillan}%
      }
      \strng{namehash}{416c234e34c8082fb7acf86c6e7a499a}
      \strng{fullhash}{7d301d11b9579ee16fad350195f2d756}
      \strng{fullhashraw}{7d301d11b9579ee16fad350195f2d756}
      \strng{bibnamehash}{416c234e34c8082fb7acf86c6e7a499a}
      \strng{authorbibnamehash}{416c234e34c8082fb7acf86c6e7a499a}
      \strng{authornamehash}{416c234e34c8082fb7acf86c6e7a499a}
      \strng{authorfullhash}{7d301d11b9579ee16fad350195f2d756}
      \strng{authorfullhashraw}{7d301d11b9579ee16fad350195f2d756}
      \strng{editorbibnamehash}{d1f1309f75dc90b7a1846a2efbd43572}
      \strng{editornamehash}{d1f1309f75dc90b7a1846a2efbd43572}
      \strng{editorfullhash}{519612891addebf4b3e5e61fefc6d52d}
      \strng{editorfullhashraw}{519612891addebf4b3e5e61fefc6d52d}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
      \field{labeltitlesource}{title}
      \field{note}{A note}
      \field{title}{Very Long Title with XDATA}
      \annotation{field}{note}{default}{}{}{0}{bignote}
      \annotation{item}{author}{default}{1}{}{0}{biggerauthor}
      \annotation{item}{author}{default}{2}{}{0}{bigauthor}
      \annotation{item}{author}{default}{3}{}{0}{bigishauthor}
      \annotation{item}{editor}{default}{2}{}{0}{bigishauthor}
      \annotation{item}{editor}{default}{4}{}{0}{bigauthor}
      \annotation{item}{institution}{default}{2}{}{0}{biginst}
      \annotation{item}{location}{default}{2}{}{0}{bigloc}
      \annotation{item}{location}{default}{3}{}{0}{bigloc}
      \annotation{item}{publisher}{default}{1}{}{0}{bigpublisher}
    \endentry
"#;

const EXPECTED_W1: &[&str] = &[
    r#"book entry 'gxd1' has XDATA reference from field 'publisher' that contains no source field (section 0)"#,
    r#"book entry 'gxd1' has XDATA reference from field 'addendum' that contains no source field (section 0)"#,
    r#"Field 'note' in book entry 'gxd1' references XDATA field 'note' in entry 'gxd2' and this field does not exist, not resolving (section 0)"#,
    r#"Field 'translator' in book entry 'gxd1' references field 'author' position 3 in entry 'gxd2' and this position does not exist, not resolving (section 0)"#,
    r#"Field 'lista' in book entry 'gxd1' references field 'location' position 5 in entry 'gxd3' and this position does not exist, not resolving (section 0)"#,
    r#"Field 'organization' in book entry 'gxd1' which xdata references field 'author' in entry 'gxd2' are not the same types, not resolving (section 0)"#,
    r#"book entry 'gxd1' references XDATA entry 'lxd1' which is not an XDATA entry, not resolving (section 0)"#,
];

const EXPECTED_W2: &[&str] = &[
    r#"book entry 'bltxgxd1' has XDATA reference from field 'publisher' that contains no source field (section 0)"#,
    r#"book entry 'bltxgxd1' has XDATA reference from field 'addendum' that contains no source field (section 0)"#,
    r#"Field 'translator' in book entry 'bltxgxd1' references field 'author' position 3 in entry 'bltxgxd2' and this position does not exist, not resolving (section 0)"#,
    r#"Field 'lista' in book entry 'bltxgxd1' references field 'location' position 5 in entry 'bltxgxd3' and this position does not exist, not resolving (section 0)"#,
    r#"Field 'organization' in book entry 'bltxgxd1' which xdata references field 'author' in entry 'bltxgxd2' are not the same types, not resolving (section 0)"#,
    r#"Field 'note' in book entry 'bltxgxd1' references XDATA field 'note' in entry 'bltxgxd2' and this field does not exist, not resolving (section 0)"#,
];

fn entry_diagnostics<'a>(result: &'a bib_engine::BibResult, key: &str) -> Vec<&'a str> {
    result
        .diagnostics()
        .filter(|diagnostic| {
            diagnostic
                .entry()
                .is_some_and(|entry| entry.as_str() == key)
        })
        .map(|diagnostic| diagnostic.message())
        .collect()
}

fn rendered_diagnostics(result: &bib_engine::BibResult) -> Vec<String> {
    result
        .diagnostics()
        .map(|diagnostic| {
            format!("{:?} - {}", diagnostic.severity(), diagnostic.message()).to_uppercase()
        })
        .collect()
}

#[test]
#[ignore = "xfail: Biber XDATA inheritance/output is not implemented by bib-engine"]
fn assertion_001_xdata_test_1() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xd1"))
            .as_deref(),
        Some(EXPECTED_XD1)
    );
}

#[test]
#[ignore = "xfail: Biber XDATA inheritance/output is not implemented by bib-engine"]
fn assertion_002_xdata_test_2() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xd2"))
            .as_deref(),
        Some(EXPECTED_XD2)
    );
}

#[test]
fn assertion_003_xdata_test_3() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "macmillan")),
        None
    );
}

#[test]
fn assertion_004_xdata_test_4() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "macmillan:pub")),
        None
    );
}

#[test]
#[ignore = "xfail: Biber XDATA inheritance/output is not implemented by bib-engine"]
fn assertion_005_xdata_granular_test_1() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "gxd1"))
            .as_deref(),
        Some(EXPECTED_GXD1)
    );
}

#[test]
#[ignore = "xfail: Biber XDATA inheritance/output is not implemented by bib-engine"]
fn assertion_006_xdata_granular_test_2() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "gxd1g"))
            .as_deref(),
        Some(EXPECTED_GXD1G)
    );
}

#[test]
#[ignore = "xfail: Biber XDATA inheritance/output is not implemented by bib-engine"]
fn assertion_007_xdata_granular_test_3() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "bltxgxd1"))
            .as_deref(),
        Some(EXPECTED_BLTXGXD1)
    );
}

#[test]
#[ignore = "xfail: Biber XDATA inheritance/output is not implemented by bib-engine"]
fn assertion_008_xdata_annotation_test_1() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .and_then(|result| output_entry(result, "xdann1"))
            .as_deref(),
        Some(EXPECTED_XDANN1)
    );
}

#[test]
#[ignore = "xfail: Biber circular-XDATA diagnostics are not implemented by bib-engine"]
fn assertion_009_cyclic_xdata_error_check_1() {
    let result = try_run_fixture("xdata");
    assert!(
        result
            .as_ref()
            .ok()
            .map(rendered_diagnostics)
            .unwrap_or_default()
            .iter()
            .any(|diagnostic| diagnostic
                == "ERROR - CIRCULAR XDATA INHERITANCE BETWEEN 'LXD1:LOOP'<->'LXD2:LOOP'")
    );
}

#[test]
#[ignore = "xfail: Biber circular-XDATA diagnostics are not implemented by bib-engine"]
fn assertion_010_cyclic_xdata_error_check_2() {
    let result = try_run_fixture("xdata");
    assert!(
        result
            .as_ref()
            .ok()
            .map(rendered_diagnostics)
            .unwrap_or_default()
            .iter()
            .any(|diagnostic| diagnostic
                == "ERROR - CIRCULAR XDATA INHERITANCE BETWEEN 'LXD4:LOOP'<->'LXD4:LOOP'")
    );
}

#[test]
#[ignore = "xfail: Biber circular-XDATA diagnostics are not implemented by bib-engine"]
fn assertion_011_cyclic_xdata_error_check_3() {
    let result = try_run_fixture("xdata");
    assert!(
        result
            .as_ref()
            .ok()
            .map(rendered_diagnostics)
            .unwrap_or_default()
            .iter()
            .any(|diagnostic| diagnostic
                == "ERROR - CIRCULAR XDATA INHERITANCE BETWEEN 'LOOP'<->'LOOP:3'")
    );
}

#[test]
#[ignore = "xfail: Biber granular-XDATA diagnostics are not implemented by bib-engine"]
fn assertion_012_granular_xdata_resolution_warnings_bibtex() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| entry_diagnostics(result, "gxd1"))
            .unwrap_or_default(),
        EXPECTED_W1
    );
}

#[test]
#[ignore = "xfail: Biber granular-XDATA diagnostics are not implemented by bib-engine"]
fn assertion_013_granular_xdata_resolution_warnings_biblatexml() {
    let result = try_run_fixture("xdata");
    assert_eq!(
        result
            .as_ref()
            .ok()
            .map(|result| entry_diagnostics(result, "bltxgxd1"))
            .unwrap_or_default(),
        EXPECTED_W2
    );
}
