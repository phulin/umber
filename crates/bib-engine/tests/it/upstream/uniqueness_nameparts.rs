// Native Rust translation of upstream t/uniqueness-nameparts.t at commit 74252e6.

use std::path::PathBuf;

use bib_engine::{
    BibAttempt, BibJob, BibOptionsBuilder, BibSession, EntryId, FieldId, FieldValue,
    FileProvisioner, OutputFormat, OutputRequest, ResolvedFile, SectionId, VfsLimits, VirtualPath,
};

struct FixtureResult {
    document: bib_engine::ProcessedBibliography,
    bbl: String,
}

fn process_fixture() -> FixtureResult {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/bib/upstream-2.22/tdata");
    let control_name = "uniqueness-nameparts.bcf";
    let control = VirtualPath::user(control_name).expect("valid control path");
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("valid VFS limits");
    provisioner
        .register_user(
            control.clone(),
            std::fs::read(fixture_dir.join(control_name)).expect("committed BCF fixture"),
        )
        .expect("unique control file");
    let mut options = BibOptionsBuilder::new();
    options
        .output(OutputRequest::new(
            VirtualPath::user("native.bbl").expect("valid output path"),
            OutputFormat::Bbl,
        ))
        .expect("unique output");
    let job = BibJob::new(control, options.freeze());
    let mut session = BibSession::default();
    loop {
        match session.process(&job, &provisioner.snapshot()) {
            BibAttempt::Complete(result) => {
                let bbl = result
                    .files()
                    .find(|file| file.path().as_str().ends_with("native.bbl"))
                    .map(|file| String::from_utf8_lossy(file.bytes()).into_owned())
                    .unwrap_or_default();
                return FixtureResult {
                    document: result.document().as_ref().clone(),
                    bbl,
                };
            }
            BibAttempt::NeedResources(requests) => {
                provisioner.expect(&requests);
                for request in requests
                    .required
                    .iter()
                    .chain(requests.prefetch_hints.iter())
                {
                    let path = fixture_dir.join(request.key().name());
                    if !path.is_file() {
                        continue;
                    }
                    provisioner
                        .provision(ResolvedFile {
                            request: request.key().clone(),
                            virtual_path: format!("/texlive/bib/{}", request.key().name()).into(),
                            bytes: std::fs::read(path).expect("committed requested fixture"),
                            expected_digest: None,
                        })
                        .expect("requested fixture is valid");
                }
            }
            BibAttempt::Failed(failure) => panic!("fixture processing failed: {failure:?}"),
        }
    }
}

fn name_metadata(entry_key: &str, assignment_key: &str) -> Vec<String> {
    let fixture = process_fixture();
    let entry = fixture
        .document
        .section(SectionId::new(0))
        .and_then(|section| section.entry(&EntryId::new(entry_key).expect("valid entry key")))
        .expect("fixture entry exists");
    let names = match entry
        .fields()
        .get(&FieldId::new("author").expect("valid field name"))
        .expect("author name list exists")
    {
        FieldValue::NameList(names) => names,
        value => panic!("expected author name list, got {value:?}"),
    };
    names
        .iter()
        .next()
        .expect("first author exists")
        .assignments()
        .filter(|assignment| assignment.key() == assignment_key)
        .map(|assignment| assignment.value().to_owned())
        .collect()
}

fn output_entry(entry_key: &str, occurrence: usize) -> String {
    let fixture = process_fixture();
    let marker = format!("\\\\entry{{{entry_key}}}");
    let marker_at = fixture
        .bbl
        .match_indices(&marker)
        .nth(occurrence)
        .map(|(offset, _)| offset)
        .expect("entry is present in generated BBL");
    let start = fixture.bbl[..marker_at].rfind("    ").unwrap_or(marker_at);
    let end = fixture.bbl[marker_at..]
        .find("\\\\endentry")
        .map(|offset| marker_at + offset + "\\\\endentry".len())
        .expect("entry is terminated");
    fixture.bbl[start..end].to_owned()
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_001_uniquename_namepart_1() {
    assert_eq!(
        output_entry(r#####"un1"#####, 0),
        r#####"    \entry{un1}{article}{}{}
      \name{author}{1}{}{%
        {{un=1,uniquepart=middle,hash=329d8f9192ea3349d700160c9ddb505d}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=1,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=1}}%
      }
      \strng{namehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \strng{bibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorbibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authornamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \field{labelalpha}{SmiJohSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_002_uniquename_namepart_2() {
    assert_eq!(
        output_entry(r#####"un2"#####, 0),
        r#####"    \entry{un2}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=7551114aede4ef69e4b3683039801706}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=1,
           middle={Alan},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhashraw}{7551114aede4ef69e4b3683039801706}
      \strng{bibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorbibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authornamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhashraw}{7551114aede4ef69e4b3683039801706}
      \field{labelalpha}{SmiJohAla}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_003_uniquename_namepart_3() {
    assert_eq!(
        output_entry(r#####"un3"#####, 0),
        r#####"    \entry{un3}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=401aebda288799a7c757526242d8c9fc}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=1,
           middle={Arthur},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhashraw}{401aebda288799a7c757526242d8c9fc}
      \strng{bibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorbibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authornamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhashraw}{401aebda288799a7c757526242d8c9fc}
      \field{labelalpha}{SmiJohArt}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_004_uniquename_namepart_4() {
    assert_eq!(
        output_entry(r#####"un4"#####, 0),
        r#####"    \entry{un4}{article}{}{}
      \name{author}{1}{}{%
        {{un=1,uniquepart=given,hash=f6038a264619efefd49c7daac56424ca}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Alan},
           giveni={A\bibinitperiod},
           givenun=1,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhashraw}{f6038a264619efefd49c7daac56424ca}
      \strng{bibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorbibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authornamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhashraw}{f6038a264619efefd49c7daac56424ca}
      \field{labelalpha}{SmiAlaSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_005_uniquename_namepart_5() {
    assert_eq!(
        output_entry(r#####"un1"#####, 1),
        r#####"    \entry{un1}{article}{}{}
      \name{author}{1}{}{%
        {{un=1,uniquepart=middle,hash=329d8f9192ea3349d700160c9ddb505d}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=2,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=1}}%
      }
      \strng{namehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \strng{bibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorbibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authornamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \field{labelalpha}{SmiJohSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_006_uniquename_namepart_6() {
    assert_eq!(
        output_entry(r#####"un2"#####, 1),
        r#####"    \entry{un2}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=7551114aede4ef69e4b3683039801706}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=2,
           middle={Alan},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhashraw}{7551114aede4ef69e4b3683039801706}
      \strng{bibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorbibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authornamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhashraw}{7551114aede4ef69e4b3683039801706}
      \field{labelalpha}{SmiJohAla}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_007_uniquename_namepart_7() {
    assert_eq!(
        output_entry(r#####"un3"#####, 1),
        r#####"    \entry{un3}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=401aebda288799a7c757526242d8c9fc}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=2,
           middle={Arthur},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhashraw}{401aebda288799a7c757526242d8c9fc}
      \strng{bibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorbibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authornamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhashraw}{401aebda288799a7c757526242d8c9fc}
      \field{labelalpha}{SmiJohArt}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_008_uniquename_namepart_8() {
    assert_eq!(
        output_entry(r#####"un4"#####, 1),
        r#####"    \entry{un4}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=given,hash=f6038a264619efefd49c7daac56424ca}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Alan},
           giveni={A\bibinitperiod},
           givenun=2,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhashraw}{f6038a264619efefd49c7daac56424ca}
      \strng{bibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorbibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authornamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhashraw}{f6038a264619efefd49c7daac56424ca}
      \field{labelalpha}{SmiAlaSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_009_uniquename_metadata_1() {
    assert_eq!(
        name_metadata(r#####"un1"#####, r#####"namestring-current"#####),
        vec![r#####"SmithSimon"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_010_uniquename_metadata_2() {
    assert_eq!(
        name_metadata(r#####"un1"#####, r#####"namestring"#####),
        vec![
            r#####"Smith"#####.to_owned(),
            r#####"SmithS"#####.to_owned(),
            r#####"SmithSimon"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_011_uniquename_metadata_3() {
    assert_eq!(
        name_metadata(r#####"un1"#####, r#####"namedisschema"#####),
        vec![
            r#####"base:family"#####.to_owned(),
            r#####"middle:init"#####.to_owned(),
            r#####"middle:full"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_012_uniquename_metadata_4() {
    assert_eq!(
        name_metadata(r#####"un2"#####, r#####"namestring-current"#####),
        vec![r#####"SmithAlan"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_013_uniquename_metadata_5() {
    assert_eq!(
        name_metadata(r#####"un2"#####, r#####"namestring"#####),
        vec![
            r#####"Smith"#####.to_owned(),
            r#####"SmithA"#####.to_owned(),
            r#####"SmithAlan"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_014_uniquename_metadata_6() {
    assert_eq!(
        name_metadata(r#####"un2"#####, r#####"namedisschema"#####),
        vec![
            r#####"base:family"#####.to_owned(),
            r#####"middle:init"#####.to_owned(),
            r#####"middle:full"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_015_uniquename_metadata_7() {
    assert_eq!(
        name_metadata(r#####"un3"#####, r#####"namestring-current"#####),
        vec![r#####"SmithArthur"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_016_uniquename_metadata_8() {
    assert_eq!(
        name_metadata(r#####"un3"#####, r#####"namestring"#####),
        vec![
            r#####"Smith"#####.to_owned(),
            r#####"SmithA"#####.to_owned(),
            r#####"SmithArthur"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_017_uniquename_metadata_9() {
    assert_eq!(
        name_metadata(r#####"un3"#####, r#####"namedisschema"#####),
        vec![
            r#####"base:family"#####.to_owned(),
            r#####"middle:init"#####.to_owned(),
            r#####"middle:full"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_018_uniquename_metadata_10() {
    assert_eq!(
        name_metadata(r#####"un4"#####, r#####"namestring-current"#####),
        vec![r#####"SmithSimon"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_019_uniquename_metadata_11() {
    assert_eq!(
        name_metadata(r#####"un4"#####, r#####"namestring"#####),
        vec![
            r#####"Smith"#####.to_owned(),
            r#####"SmithS"#####.to_owned(),
            r#####"SmithSimon"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_020_uniquename_metadata_12() {
    assert_eq!(
        name_metadata(r#####"un4"#####, r#####"namedisschema"#####),
        vec![
            r#####"base:family"#####.to_owned(),
            r#####"middle:init"#####.to_owned(),
            r#####"middle:full"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_021_uniquename_metadata_13() {
    assert_eq!(
        name_metadata(r#####"un5"#####, r#####"namestring-current"#####),
        vec![r#####"SmithSimon"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_022_uniquename_metadata_14() {
    assert_eq!(
        name_metadata(r#####"un5"#####, r#####"namestring"#####),
        vec![
            r#####"Smith"#####.to_owned(),
            r#####"SmithSimon"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_023_uniquename_metadata_15() {
    assert_eq!(
        name_metadata(r#####"un5"#####, r#####"namedisschema"#####),
        vec![
            r#####"base:family"#####.to_owned(),
            r#####"middle:fullonly"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_024_uniquename_metadata_16() {
    assert_eq!(
        name_metadata(r#####"un6"#####, r#####"namestring-current"#####),
        vec![r#####"SmithSmythe"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_025_uniquename_metadata_17() {
    assert_eq!(
        name_metadata(r#####"un6"#####, r#####"namestring"#####),
        vec![
            r#####"Smith"#####.to_owned(),
            r#####"SmithS"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_026_uniquename_metadata_18() {
    assert_eq!(
        name_metadata(r#####"un6"#####, r#####"namedisschema"#####),
        vec![
            r#####"base:family"#####.to_owned(),
            r#####"middle:init"#####.to_owned()
        ]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_027_uniquename_metadata_19() {
    assert_eq!(
        name_metadata(r#####"un7"#####, r#####"namestring-current"#####),
        vec![r#####"Smith"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_028_uniquename_metadata_20() {
    assert_eq!(
        name_metadata(r#####"un7"#####, r#####"namestring"#####),
        vec![r#####"Smith"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_029_uniquename_metadata_21() {
    assert_eq!(
        name_metadata(r#####"un7"#####, r#####"namedisschema"#####),
        vec![r#####"base:family"#####.to_owned()]
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_030_uniquename_namepart_9() {
    assert_eq!(
        output_entry(r#####"un1"#####, 2),
        r#####"    \entry{un1}{article}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=329d8f9192ea3349d700160c9ddb505d}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{fullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \strng{bibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorbibnamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authornamehash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhash}{329d8f9192ea3349d700160c9ddb505d}
      \strng{authorfullhashraw}{329d8f9192ea3349d700160c9ddb505d}
      \field{extraname}{5}
      \field{labelalpha}{SmiJohSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{5}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_031_uniquename_namepart_10() {
    assert_eq!(
        output_entry(r#####"un2"#####, 2),
        r#####"    \entry{un2}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=7551114aede4ef69e4b3683039801706}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           middle={Alan},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhash}{7551114aede4ef69e4b3683039801706}
      \strng{fullhashraw}{7551114aede4ef69e4b3683039801706}
      \strng{bibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorbibnamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authornamehash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhash}{7551114aede4ef69e4b3683039801706}
      \strng{authorfullhashraw}{7551114aede4ef69e4b3683039801706}
      \field{labelalpha}{SmiJohAla}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_032_uniquename_namepart_11() {
    assert_eq!(
        output_entry(r#####"un3"#####, 2),
        r#####"    \entry{un3}{article}{}{}
      \name{author}{1}{}{%
        {{un=2,uniquepart=middle,hash=401aebda288799a7c757526242d8c9fc}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={John},
           giveni={J\bibinitperiod},
           givenun=0,
           middle={Arthur},
           middlei={A\bibinitperiod},
           middleun=2}}%
      }
      \strng{namehash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{fullhashraw}{401aebda288799a7c757526242d8c9fc}
      \strng{bibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorbibnamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authornamehash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhash}{401aebda288799a7c757526242d8c9fc}
      \strng{authorfullhashraw}{401aebda288799a7c757526242d8c9fc}
      \field{labelalpha}{SmiJohArt}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_033_uniquename_namepart_12() {
    assert_eq!(
        output_entry(r#####"un4"#####, 2),
        r#####"    \entry{un4}{article}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=f6038a264619efefd49c7daac56424ca}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Alan},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{fullhashraw}{f6038a264619efefd49c7daac56424ca}
      \strng{bibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorbibnamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authornamehash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhash}{f6038a264619efefd49c7daac56424ca}
      \strng{authorfullhashraw}{f6038a264619efefd49c7daac56424ca}
      \field{extraname}{1}
      \field{labelalpha}{SmiAlaSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{1}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_034_uniquename_namepart_13() {
    assert_eq!(
        output_entry(r#####"un5"#####, 2),
        r#####"    \entry{un5}{article}{uniquenametemplatename=test3}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,hash=74fba0d07ca65976bbff1034f9bb22e6}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Simon},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{fullhash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{fullhashraw}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{bibnamehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authorbibnamehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authornamehash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authorfullhash}{74fba0d07ca65976bbff1034f9bb22e6}
      \strng{authorfullhashraw}{74fba0d07ca65976bbff1034f9bb22e6}
      \field{extraname}{2}
      \field{labelalpha}{SmiArtSim}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{2}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_035_uniquename_namepart_14() {
    assert_eq!(
        output_entry(r#####"un6"#####, 2),
        r#####"    \entry{un6}{article}{}{}
      \name{author}{1}{uniquenametemplatename=test4}{%
        {{un=0,uniquepart=base,hash=8100e7d06d05938e91bf8863f5c20e33}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Smythe},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{fullhash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{fullhashraw}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{bibnamehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authorbibnamehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authornamehash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authorfullhash}{8100e7d06d05938e91bf8863f5c20e33}
      \strng{authorfullhashraw}{8100e7d06d05938e91bf8863f5c20e33}
      \field{extraname}{3}
      \field{labelalpha}{SmiArtSmy}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{3}
      \field{labelnamesource}{author}
    \\endentry
"#####
    );
}

#[test]
#[ignore = "xfail: name-part disambiguation metadata differs from the Biber 2.22 expectation"]
fn assertion_036_uniquename_namepart_15() {
    assert_eq!(
        output_entry(r#####"un7"#####, 2),
        r#####"    \entry{un7}{article}{}{}
      \name{author}{1}{}{%
        {{un=0,uniquepart=base,uniquenametemplatename=test5,hash=c21736158273b6f2f368818459734e04}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod},
           givenun=0,
           middle={Smedley},
           middlei={S\bibinitperiod},
           middleun=0}}%
      }
      \strng{namehash}{c21736158273b6f2f368818459734e04}
      \strng{fullhash}{c21736158273b6f2f368818459734e04}
      \strng{fullhashraw}{c21736158273b6f2f368818459734e04}
      \strng{bibnamehash}{c21736158273b6f2f368818459734e04}
      \strng{authorbibnamehash}{c21736158273b6f2f368818459734e04}
      \strng{authornamehash}{c21736158273b6f2f368818459734e04}
      \strng{authorfullhash}{c21736158273b6f2f368818459734e04}
      \strng{authorfullhashraw}{c21736158273b6f2f368818459734e04}
      \field{extraname}{4}
      \field{labelalpha}{SmiArtSme}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{extradate}{4}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}
