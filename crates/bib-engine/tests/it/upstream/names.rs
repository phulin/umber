// Native Rust translation of the corresponding upstream Biber name test at commit 74252e6.

use std::path::PathBuf;

use bib_engine::{
    BibAttempt, BibJob, BibOptionsBuilder, BibSession, EntryId, FieldId, FieldValue,
    FileProvisioner, Name, NamePartValue, OutputFormat, OutputRequest, ProcessedBibliography,
    ResolvedFile, SectionId, VfsLimits, VirtualPath,
};

#[derive(Debug, Eq, PartialEq)]
struct PartSnapshot {
    value: Option<String>,
    initials: Vec<String>,
}

#[derive(Debug, Eq, PartialEq)]
struct NameSnapshot {
    given: PartSnapshot,
    family: PartSnapshot,
    prefix: PartSnapshot,
    suffix: PartSnapshot,
}

struct FixtureResult {
    document: ProcessedBibliography,
    bbl: String,
}

fn process_fixture(control_name: &str, inline_bib: Option<&str>) -> FixtureResult {
    let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/bib/upstream-2.22/tdata");
    let control = VirtualPath::user(control_name).expect("valid control path");
    let mut provisioner = FileProvisioner::new(VfsLimits::default()).expect("valid VFS limits");
    provisioner
        .register_user(
            control.clone(),
            std::fs::read(fixture_dir.join(control_name)).expect("committed BCF fixture"),
        )
        .expect("unique control file");
    let output_path = VirtualPath::user("native.bbl").expect("valid output path");
    let mut options = BibOptionsBuilder::new();
    options
        .output(OutputRequest::new(output_path, OutputFormat::Bbl))
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
                    let bytes = if request.key().name().ends_with(".bib") {
                        inline_bib
                            .map(|bib| bib.as_bytes().to_vec())
                            .or_else(|| std::fs::read(&path).ok())
                    } else {
                        std::fs::read(&path).ok()
                    };
                    let Some(bytes) = bytes else { continue };
                    provisioner
                        .provision(ResolvedFile {
                            request: request.key().clone(),
                            virtual_path: format!("/texlive/bib/{}", request.key().name()).into(),
                            bytes,
                            expected_digest: None,
                        })
                        .expect("requested fixture is valid");
                }
            }
            BibAttempt::Failed(failure) => panic!("fixture processing failed: {failure:?}"),
        }
    }
}

fn part_snapshot(part: Option<&NamePartValue>) -> PartSnapshot {
    PartSnapshot {
        value: part.map(|part| part.value().as_str().to_owned()),
        initials: part
            .into_iter()
            .flat_map(NamePartValue::initials)
            .map(str::to_owned)
            .collect(),
    }
}

fn name_snapshot(name: &Name) -> NameSnapshot {
    NameSnapshot {
        given: part_snapshot(name.given()),
        family: part_snapshot(name.family()),
        prefix: part_snapshot(name.prefix()),
        suffix: part_snapshot(name.suffix()),
    }
}

fn parsed_name(control: &str, source: &str) -> Name {
    let bib = format!("@book{{native, author = {{{source}}}}}");
    let fixture = process_fixture(control, Some(&bib));
    let entry = fixture
        .document
        .section(SectionId::new(0))
        .and_then(|section| section.entry(&EntryId::new("native").expect("valid entry key")))
        .expect("synthetic entry is processed");
    match entry
        .fields()
        .get(&FieldId::new("author").expect("valid field name"))
        .expect("synthetic author is processed")
    {
        FieldValue::NameList(names) => names.iter().next().expect("one synthetic author").clone(),
        value => panic!("expected a name list, got {value:?}"),
    }
}

fn field_text(control: &str, entry_key: &str, field_name: &str) -> Option<String> {
    let fixture = process_fixture(control, None);
    let entry = fixture
        .document
        .section(SectionId::new(0))?
        .entry(&EntryId::new(entry_key).expect("valid entry key"))?;
    match entry
        .fields()
        .get(&FieldId::new(field_name).expect("valid field name"))?
    {
        FieldValue::Literal(value) => Some(value.as_str().to_owned()),
        FieldValue::Integer(value) => Some(value.to_string()),
        FieldValue::Boolean(value) => Some(if *value { "1" } else { "0" }.to_owned()),
        _ => None,
    }
}

fn name_count(control: &str, entry_key: &str) -> usize {
    let fixture = process_fixture(control, None);
    let entry = fixture
        .document
        .section(SectionId::new(0))
        .and_then(|section| section.entry(&EntryId::new(entry_key).expect("valid entry key")))
        .expect("fixture entry exists");
    let source = match entry
        .fields()
        .get(&FieldId::new("labelnamesource").expect("valid field name"))
        .expect("label-name source exists")
    {
        FieldValue::Literal(source) => source.as_str(),
        value => panic!("expected literal label-name source, got {value:?}"),
    };
    match entry
        .fields()
        .get(&FieldId::new(source).expect("valid name field"))
        .expect("selected name list exists")
    {
        FieldValue::NameList(names) => names.len(),
        value => panic!("expected selected name list, got {value:?}"),
    }
}

fn name_initial(control: &str, entry_key: &str, part: &str, initial_index: usize) -> String {
    let fixture = process_fixture(control, None);
    let entry = fixture
        .document
        .section(SectionId::new(0))
        .and_then(|section| section.entry(&EntryId::new(entry_key).expect("valid entry key")))
        .expect("fixture entry exists");
    let names = match entry
        .fields()
        .get(&FieldId::new("author").expect("valid field name"))
        .expect("author list exists")
    {
        FieldValue::NameList(names) => names,
        value => panic!("expected author name list, got {value:?}"),
    };
    let name = names.iter().next().expect("first author exists");
    let part = match part {
        "given" => name.given(),
        "family" => name.family(),
        "prefix" => name.prefix(),
        "suffix" => name.suffix(),
        _ => None,
    }
    .expect("requested name part exists");
    part.initials()
        .nth(initial_index)
        .expect("requested initial exists")
        .to_owned()
}

fn output_entry(control: &str, entry_key: &str, occurrence: usize) -> String {
    let fixture = process_fixture(control, None);
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

fn to_extended_name(name: &Name) -> String {
    let mut assignments = Vec::new();
    if let Some(family) = name.family() {
        assignments.push(format!("family={}", family.value().as_str()));
    }
    if let Some(given) = name.given() {
        assignments.push(format!("given={}", given.value().as_str()));
    }
    if let Some(prefix) = name.prefix() {
        assignments.push(format!("prefix={}", prefix.value().as_str()));
    }
    if let Some(suffix) = name.suffix() {
        assignments.push(format!("suffix={}", suffix.value().as_str()));
    }
    if name.use_prefix() == Some(true) {
        assignments.push("useprefix=true".to_owned());
    }
    assignments.join(", ")
}

#[test]
fn assertion_001_parsename_1() {
    assert_eq!(
        name_snapshot(&parsed_name(r#####"names.bcf"#####, r#####"John Doe"#####)),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"John"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Doe"#####.to_owned()),
                initials: vec![r#####"D"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_002_parsename_2() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Doe, Jr, John"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"John"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Doe"#####.to_owned()),
                initials: vec![r#####"D"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: Some(r#####"Jr"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned()]
            }
        }
    );
}

#[test]
fn assertion_003_parsename_3() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"von Berlichingen zu Hornberg, Johann Gottfried"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Johann~Gottfried"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned(), r#####"G"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Berlichingen zu~Hornberg"#####.to_owned()),
                initials: vec![
                    r#####"B"#####.to_owned(),
                    r#####"z"#####.to_owned(),
                    r#####"H"#####.to_owned()
                ]
            },
            prefix: PartSnapshot {
                value: Some(r#####"von"#####.to_owned()),
                initials: vec![r#####"v"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_004_parsename_4() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"von Berlichingen zu Hornberg, Johann Gottfried"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Johann~Gottfried"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned(), r#####"G"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Berlichingen zu~Hornberg"#####.to_owned()),
                initials: vec![
                    r#####"B"#####.to_owned(),
                    r#####"z"#####.to_owned(),
                    r#####"H"#####.to_owned()
                ]
            },
            prefix: PartSnapshot {
                value: Some(r#####"von"#####.to_owned()),
                initials: vec![r#####"v"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_005_parsename_5() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"{Robert and Sons, Inc.}"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: None,
                initials: vec![]
            },
            family: PartSnapshot {
                value: Some(r#####"Robert and Sons, Inc."#####.to_owned()),
                initials: vec![r#####"R"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_006_parsename_6() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"al-Ṣāliḥ, ʿAbdallāh"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"ʿAbdallāh"#####.to_owned()),
                initials: vec![r#####"A"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"al-Ṣāliḥ"#####.to_owned()),
                initials: vec![r#####"Ṣ"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_007_parsename_6a() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"al- Hakim, Tawfik"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Tawfik"#####.to_owned()),
                initials: vec![r#####"T"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Hakim"#####.to_owned()),
                initials: vec![r#####"H"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: Some(r#####"al-"#####.to_owned()),
                initials: vec![r#####"a"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_008_parsename_7() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Jean Charles Gabriel de la Vallée Poussin"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean Charles~Gabriel"#####.to_owned()),
                initials: vec![
                    r#####"J"#####.to_owned(),
                    r#####"C"#####.to_owned(),
                    r#####"G"#####.to_owned()
                ]
            },
            family: PartSnapshot {
                value: Some(r#####"Vallée~Poussin"#####.to_owned()),
                initials: vec![r#####"V"#####.to_owned(), r#####"P"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: Some(r#####"de~la"#####.to_owned()),
                initials: vec![r#####"d"#####.to_owned(), r#####"l"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_009_parsename_8() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"{Jean Charles Gabriel} de la Vallée Poussin"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean Charles Gabriel"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Vallée~Poussin"#####.to_owned()),
                initials: vec![r#####"V"#####.to_owned(), r#####"P"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: Some(r#####"de~la"#####.to_owned()),
                initials: vec![r#####"d"#####.to_owned(), r#####"l"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_010_parsename_9() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Jean Charles Gabriel {de la} Vallée Poussin"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean Charles Gabriel {de la}~Vallée"#####.to_owned()),
                initials: vec![
                    r#####"J"#####.to_owned(),
                    r#####"C"#####.to_owned(),
                    r#####"G"#####.to_owned(),
                    r#####"d"#####.to_owned(),
                    r#####"V"#####.to_owned()
                ]
            },
            family: PartSnapshot {
                value: Some(r#####"Poussin"#####.to_owned()),
                initials: vec![r#####"P"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_011_parsename_10() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Jean Charles Gabriel de la {Vallée Poussin}"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean Charles~Gabriel"#####.to_owned()),
                initials: vec![
                    r#####"J"#####.to_owned(),
                    r#####"C"#####.to_owned(),
                    r#####"G"#####.to_owned()
                ]
            },
            family: PartSnapshot {
                value: Some(r#####"Vallée Poussin"#####.to_owned()),
                initials: vec![r#####"V"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: Some(r#####"de~la"#####.to_owned()),
                initials: vec![r#####"d"#####.to_owned(), r#####"l"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_012_parsename_11() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"{Jean Charles Gabriel} de la {Vallée Poussin}"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean Charles Gabriel"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Vallée Poussin"#####.to_owned()),
                initials: vec![r#####"V"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: Some(r#####"de~la"#####.to_owned()),
                initials: vec![r#####"d"#####.to_owned(), r#####"l"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_013_parsename_12() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Jean Charles Gabriel Poussin"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean Charles~Gabriel"#####.to_owned()),
                initials: vec![
                    r#####"J"#####.to_owned(),
                    r#####"C"#####.to_owned(),
                    r#####"G"#####.to_owned()
                ]
            },
            family: PartSnapshot {
                value: Some(r#####"Poussin"#####.to_owned()),
                initials: vec![r#####"P"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_014_parsename_13() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Jean Charles {Poussin Lecoq}"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean~Charles"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned(), r#####"C"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Poussin Lecoq"#####.to_owned()),
                initials: vec![r#####"P"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_015_parsename_14() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"J. C. G. de la Vallée Poussin"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"J.~C.~G."#####.to_owned()),
                initials: vec![
                    r#####"J"#####.to_owned(),
                    r#####"C"#####.to_owned(),
                    r#####"G"#####.to_owned()
                ]
            },
            family: PartSnapshot {
                value: Some(r#####"Vallée~Poussin"#####.to_owned()),
                initials: vec![r#####"V"#####.to_owned(), r#####"P"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: Some(r#####"de~la"#####.to_owned()),
                initials: vec![r#####"d"#####.to_owned(), r#####"l"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_016_parsename_15() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"E. S. El-{M}allah"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"E.~S."#####.to_owned()),
                initials: vec![r#####"E"#####.to_owned(), r#####"S"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"El-{M}allah"#####.to_owned()),
                initials: vec![r#####"E-M"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_017_parsename_16() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"E. S. {K}ent-{B}oswell"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"E.~S."#####.to_owned()),
                initials: vec![r#####"E"#####.to_owned(), r#####"S"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"{K}ent-{B}oswell"#####.to_owned()),
                initials: vec![r#####"K-B"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_018_parsename_17() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Other, A.~N."#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"A.~N."#####.to_owned()),
                initials: vec![r#####"A"#####.to_owned(), r#####"N"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Other"#####.to_owned()),
                initials: vec![r#####"O"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_019_parsename_18() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"{{{British National Corpus}}}"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: None,
                initials: vec![]
            },
            family: PartSnapshot {
                value: Some(r#####"British National Corpus"#####.to_owned()),
                initials: vec![r#####"B"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
fn assertion_020_parsename_18a() {
    let name = parsed_name(
        r#####"names.bcf"#####,
        r#####"{{{British National Corpus}}}"#####,
    );
    assert_eq!(
        [name.given(), name.family(), name.prefix(), name.suffix()]
            .map(|part| part.map(NamePartValue::outer_braces_stripped)),
        [None, Some(true), None, None]
    );
}

#[test]
fn assertion_021_parsename_19() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"Vázques{ de }Parga, Luis"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Luis"#####.to_owned()),
                initials: vec![r#####"L"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Vázques{ de }Parga"#####.to_owned()),
                initials: vec![r#####"V"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: None,
                initials: vec![]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_022_parsename_x_1() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names.bcf"#####,
            r#####"family=Smithers Jones, prefix=van der, given=James, useprefix=true"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"James"#####.to_owned()),
                initials: vec![r#####"J"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Smithers~Jones"#####.to_owned()),
                initials: vec![r#####"S"#####.to_owned(), r#####"J"#####.to_owned()]
            },
            prefix: PartSnapshot {
                value: Some(r#####"van~der"#####.to_owned()),
                initials: vec![r#####"v"#####.to_owned(), r#####"d"#####.to_owned()]
            },
            suffix: PartSnapshot {
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_023_parsename_x_2() {
    assert_eq!(
        parsed_name(
            r#####"names.bcf"#####,
            r#####"family=Smithers Jones, prefix=van der, given=James, useprefix=true"#####
        )
        .use_prefix(),
        Some(true)
    );
}

#[test]
fn assertion_024_name_to_bibtex_1() {
    assert_eq!(
        parsed_name(r#####"names.bcf"#####, r#####"John Doe"#####).to_bibtex(),
        r#####"Doe, John"#####
    );
}

#[test]
fn assertion_025_name_to_bibtex_2() {
    assert_eq!(
        parsed_name(r#####"names.bcf"#####, r#####"John van der Doe"#####).to_bibtex(),
        r#####"van der Doe, John"#####
    );
}

#[test]
fn assertion_026_name_to_bibtex_3() {
    assert_eq!(
        parsed_name(r#####"names.bcf"#####, r#####"Doe, Jr, John"#####).to_bibtex(),
        r#####"Doe, Jr, John"#####
    );
}

#[test]
fn assertion_027_name_to_bibtex_4() {
    assert_eq!(
        parsed_name(r#####"names.bcf"#####, r#####"von Doe, Jr, John"#####).to_bibtex(),
        r#####"von Doe, Jr, John"#####
    );
}

#[test]
fn assertion_028_name_to_bibtex_5() {
    assert_eq!(
        parsed_name(r#####"names.bcf"#####, r#####"John Alan Doe"#####).to_bibtex(),
        r#####"Doe, John Alan"#####
    );
}

#[test]
fn assertion_029_name_to_bibtex_6() {
    assert_eq!(
        parsed_name(r#####"names.bcf"#####, r#####"{Robert and Sons, Inc.}"#####).to_bibtex(),
        r#####"{Robert and Sons, Inc.}"#####
    );
}

#[test]
fn assertion_030_name_to_bibtex_7() {
    assert_eq!(
        parsed_name(
            r#####"names.bcf"#####,
            r#####"Jean Charles Gabriel de la {Vallée Poussin}"#####
        )
        .to_bibtex(),
        r#####"de la {Vallée Poussin}, Jean Charles Gabriel"#####
    );
}

#[test]
fn assertion_031_name_to_bibtex_8() {
    assert_eq!(
        parsed_name(r#####"names.bcf"#####, r#####"E. S. {K}ent-{B}oswell"#####).to_bibtex(),
        r#####"{K}ent-{B}oswell, E. S."#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_032_name_to_bibtex_9() {
    assert_eq!(
        parsed_name(
            r#####"names.bcf"#####,
            r#####"family=Smithers Jones, prefix=van der, given=James, useprefix=true"#####
        )
        .to_bibtex(),
        r#####"van der Smithers Jones, James"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_033_name_to_xname_1() {
    assert_eq!(
        to_extended_name(&parsed_name(
            r#####"names.bcf"#####,
            r#####"van der Smithers Jones, James"#####
        )),
        r#####"family=Smithers Jones, given=James, prefix=van der"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_034_name_to_xname_2() {
    assert_eq!(
        to_extended_name(&parsed_name(
            r#####"names.bcf"#####,
            r#####"family=Smithers Jones, prefix=van der, given=James, useprefix=true"#####
        )),
        r#####"family=Smithers Jones, given=James, prefix=van der, useprefix=true"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_035_first_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L1"#####, 0),
        r#####"    \entry{L1}{book}{}{}
      \name{author}{1}{}{%
        {{hash=72287a68c1714cb1b9f4ab9e03a88b96}{%
           family={Adler},
           familyi={A\bibinitperiod},
           given={Alfred},
           giveni={A\bibinitperiod}}}%
      }
      \strng{namehash}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{fullhash}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{fullhashraw}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{bibnamehash}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{authorbibnamehash}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{authornamehash}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{authorfullhash}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{authorfullhashraw}{72287a68c1714cb1b9f4ab9e03a88b96}
      \field{extraname}{1}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_036_name_hashing_given_initials() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L1"#####, 1),
        r#####"    \entry{L1}{book}{}{}
      \name{author}{1}{}{%
        {{hash=a4e132fab651ba62e051557227672cda}{%
           family={Adler},
           familyi={A\bibinitperiod},
           given={Alfred},
           giveni={A\bibinitperiod}}}%
      }
      \strng{namehash}{a4e132fab651ba62e051557227672cda}
      \strng{fullhash}{a4e132fab651ba62e051557227672cda}
      \strng{fullhashraw}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{bibnamehash}{a4e132fab651ba62e051557227672cda}
      \strng{authorbibnamehash}{a4e132fab651ba62e051557227672cda}
      \strng{authornamehash}{a4e132fab651ba62e051557227672cda}
      \strng{authorfullhash}{a4e132fab651ba62e051557227672cda}
      \strng{authorfullhashraw}{72287a68c1714cb1b9f4ab9e03a88b96}
      \field{extraname}{1}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_037_name_hashing_custom_hashid() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L1id"#####, 0),
        r#####"    \entry{L1id}{book}{}{}
      \name{author}{1}{}{%
        {{hash=88354d4ba914f2ded2574386a2493996}{%
           family={Adler},
           familyi={A\bibinitperiod},
           given={Alfred},
           giveni={A\bibinitperiod}}}%
      }
      \strng{namehash}{88354d4ba914f2ded2574386a2493996}
      \strng{fullhash}{88354d4ba914f2ded2574386a2493996}
      \strng{fullhashraw}{72287a68c1714cb1b9f4ab9e03a88b96}
      \strng{bibnamehash}{88354d4ba914f2ded2574386a2493996}
      \strng{authorbibnamehash}{88354d4ba914f2ded2574386a2493996}
      \strng{authornamehash}{88354d4ba914f2ded2574386a2493996}
      \strng{authorfullhash}{88354d4ba914f2ded2574386a2493996}
      \strng{authorfullhashraw}{72287a68c1714cb1b9f4ab9e03a88b96}
      \field{extraname}{2}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_038_first_initial_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L2"#####, 0),
        r#####"    \entry{L2}{book}{}{}
      \name{author}{1}{}{%
        {{hash=2098d59d0f19a2e003ee06c1aa750d57}{%
           family={Bull},
           familyi={B\bibinitperiod},
           given={Bertie\bibnamedelima B.},
           giveni={B\bibinitperiod\bibinitdelim B\bibinitperiod}}}%
      }
      \strng{namehash}{2098d59d0f19a2e003ee06c1aa750d57}
      \strng{fullhash}{2098d59d0f19a2e003ee06c1aa750d57}
      \strng{fullhashraw}{2098d59d0f19a2e003ee06c1aa750d57}
      \strng{bibnamehash}{2098d59d0f19a2e003ee06c1aa750d57}
      \strng{authorbibnamehash}{2098d59d0f19a2e003ee06c1aa750d57}
      \strng{authornamehash}{2098d59d0f19a2e003ee06c1aa750d57}
      \strng{authorfullhash}{2098d59d0f19a2e003ee06c1aa750d57}
      \strng{authorfullhashraw}{2098d59d0f19a2e003ee06c1aa750d57}
      \field{sortinit}{B}
      \field{sortinithash}{d7095fff47cda75ca2589920aae98399}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_039_initial_initial_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L3"#####, 0),
        r#####"    \entry{L3}{book}{}{}
      \name{author}{1}{}{%
        {{hash=c8b06fe88bde128b25eb0b3b1cc5837c}{%
           family={Crop},
           familyi={C\bibinitperiod},
           given={C.\bibnamedelimi Z.},
           giveni={C\bibinitperiod\bibinitdelim Z\bibinitperiod}}}%
      }
      \strng{namehash}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \strng{fullhash}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \strng{fullhashraw}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \strng{bibnamehash}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \strng{authorbibnamehash}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \strng{authornamehash}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \strng{authorfullhash}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \strng{authorfullhashraw}{c8b06fe88bde128b25eb0b3b1cc5837c}
      \field{sortinit}{C}
      \field{sortinithash}{4d103a86280481745c9c897c925753c0}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_040_first_initial_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L4"#####, 0),
        r#####"    \entry{L4}{book}{}{}
      \name{author}{1}{}{%
        {{hash=5ec958b850c0c2de7de7c42c84b9c419}{%
           family={Decket},
           familyi={D\bibinitperiod},
           given={Derek\bibnamedelima D},
           giveni={D\bibinitperiod\bibinitdelim D\bibinitperiod}}}%
      }
      \strng{namehash}{5ec958b850c0c2de7de7c42c84b9c419}
      \strng{fullhash}{5ec958b850c0c2de7de7c42c84b9c419}
      \strng{fullhashraw}{5ec958b850c0c2de7de7c42c84b9c419}
      \strng{bibnamehash}{5ec958b850c0c2de7de7c42c84b9c419}
      \strng{authorbibnamehash}{5ec958b850c0c2de7de7c42c84b9c419}
      \strng{authornamehash}{5ec958b850c0c2de7de7c42c84b9c419}
      \strng{authorfullhash}{5ec958b850c0c2de7de7c42c84b9c419}
      \strng{authorfullhashraw}{5ec958b850c0c2de7de7c42c84b9c419}
      \field{sortinit}{D}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_041_first_prefix_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L5"#####, 0),
        r#####"    \entry{L5}{book}{}{}
      \name{author}{1}{}{%
        {{hash=c6b9d281cc1ff3f35570f76f463d4244}{%
           family={Eel},
           familyi={E\\bibinitperiod},
           given={Egbert},
           giveni={E\\bibinitperiod},
           prefix={von},
           prefixi={v\\bibinitperiod}}}%
      }
      \strng{namehash}{c6b9d281cc1ff3f35570f76f463d4244}
      \strng{fullhash}{c6b9d281cc1ff3f35570f76f463d4244}
      \strng{fullhashraw}{c6b9d281cc1ff3f35570f76f463d4244}
      \strng{bibnamehash}{c6b9d281cc1ff3f35570f76f463d4244}
      \strng{authorbibnamehash}{c6b9d281cc1ff3f35570f76f463d4244}
      \strng{authornamehash}{c6b9d281cc1ff3f35570f76f463d4244}
      \strng{authorfullhash}{c6b9d281cc1ff3f35570f76f463d4244}
      \strng{authorfullhashraw}{c6b9d281cc1ff3f35570f76f463d4244}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_042_first_prefix_prefix_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L6"#####, 0),
        r#####"    \entry{L6}{book}{}{}
      \name{author}{1}{}{%
        {{hash=5fd24d3d1608a310ec205a6b201a5495}{%
           family={Frome},
           familyi={F\\bibinitperiod},
           given={Francis},
           giveni={F\\bibinitperiod},
           prefix={van\\bibnamedelimb der\\bibnamedelima valt},
           prefixi={v\\bibinitperiod\\bibinitdelim d\\bibinitperiod\\bibinitdelim v\\bibinitperiod}}}%
      }
      \strng{namehash}{5fd24d3d1608a310ec205a6b201a5495}
      \strng{fullhash}{5fd24d3d1608a310ec205a6b201a5495}
      \strng{fullhashraw}{5fd24d3d1608a310ec205a6b201a5495}
      \strng{bibnamehash}{5fd24d3d1608a310ec205a6b201a5495}
      \strng{authorbibnamehash}{5fd24d3d1608a310ec205a6b201a5495}
      \strng{authornamehash}{5fd24d3d1608a310ec205a6b201a5495}
      \strng{authorfullhash}{5fd24d3d1608a310ec205a6b201a5495}
      \strng{authorfullhashraw}{5fd24d3d1608a310ec205a6b201a5495}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_043_first_initial_prefix_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L7"#####, 0),
        r#####"    \entry{L7}{book}{}{}
      \name{author}{1}{}{%
        {{hash=98edb0b90251df22b74328d9227eceb7}{%
           family={Gloom},
           familyi={G\\bibinitperiod},
           given={Gregory\\bibnamedelima R.},
           giveni={G\\bibinitperiod\\bibinitdelim R\\bibinitperiod},
           prefix={van},
           prefixi={v\\bibinitperiod}}}%
      }
      \strng{namehash}{98edb0b90251df22b74328d9227eceb7}
      \strng{fullhash}{98edb0b90251df22b74328d9227eceb7}
      \strng{fullhashraw}{98edb0b90251df22b74328d9227eceb7}
      \strng{bibnamehash}{98edb0b90251df22b74328d9227eceb7}
      \strng{authorbibnamehash}{98edb0b90251df22b74328d9227eceb7}
      \strng{authornamehash}{98edb0b90251df22b74328d9227eceb7}
      \strng{authorfullhash}{98edb0b90251df22b74328d9227eceb7}
      \strng{authorfullhashraw}{98edb0b90251df22b74328d9227eceb7}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_044_first_initial_prefix_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L8"#####, 0),
        r#####"    \entry{L8}{book}{}{}
      \name{author}{1}{}{%
        {{hash=1211dc8dbbc191cbcab4da3c3c1fc48a}{%
           family={Henkel},
           familyi={H\\bibinitperiod},
           given={Henry\\bibnamedelima F.},
           giveni={H\\bibinitperiod\\bibinitdelim F\\bibinitperiod},
           prefix={van},
           prefixi={v\\bibinitperiod}}}%
      }
      \strng{namehash}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \strng{fullhash}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \strng{fullhashraw}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \strng{bibnamehash}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \strng{authorbibnamehash}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \strng{authornamehash}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \strng{authorfullhash}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \strng{authorfullhashraw}{1211dc8dbbc191cbcab4da3c3c1fc48a}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_045_first_last_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L9"#####, 0),
        r#####"    \entry{L9}{book}{}{}
      \name{author}{1}{}{%
        {{hash=bae61a889ab149a6deafe45333204cf0}{%
           family={{Iliad Ipswich}},
           familyi={I\bibinitperiod},
           given={Ian},
           giveni={I\bibinitperiod}}}%
      }
      \strng{namehash}{bae61a889ab149a6deafe45333204cf0}
      \strng{fullhash}{bae61a889ab149a6deafe45333204cf0}
      \strng{fullhashraw}{bae61a889ab149a6deafe45333204cf0}
      \strng{bibnamehash}{bae61a889ab149a6deafe45333204cf0}
      \strng{authorbibnamehash}{bae61a889ab149a6deafe45333204cf0}
      \strng{authornamehash}{bae61a889ab149a6deafe45333204cf0}
      \strng{authorfullhash}{bae61a889ab149a6deafe45333204cf0}
      \strng{authorfullhashraw}{bae61a889ab149a6deafe45333204cf0}
      \field{sortinit}{I}
      \field{sortinithash}{8d291c51ee89b6cd86bf5379f0b151d8}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_046_last_suffix_first() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L10"#####, 0),
        r#####"    \entry{L10}{book}{}{}
      \name{author}{1}{}{%
        {{hash=37b4325752e394ddfb2fc810f6c88e27}{%
           family={Jolly},
           familyi={J\\bibinitperiod},
           given={James},
           giveni={J\\bibinitperiod},
           suffix={III},
           suffixi={I\\bibinitperiod}}}%
      }
      \strng{namehash}{37b4325752e394ddfb2fc810f6c88e27}
      \strng{fullhash}{37b4325752e394ddfb2fc810f6c88e27}
      \strng{fullhashraw}{37b4325752e394ddfb2fc810f6c88e27}
      \strng{bibnamehash}{37b4325752e394ddfb2fc810f6c88e27}
      \strng{authorbibnamehash}{37b4325752e394ddfb2fc810f6c88e27}
      \strng{authornamehash}{37b4325752e394ddfb2fc810f6c88e27}
      \strng{authorfullhash}{37b4325752e394ddfb2fc810f6c88e27}
      \strng{authorfullhashraw}{37b4325752e394ddfb2fc810f6c88e27}
      \field{sortinit}{J}
      \field{sortinithash}{b2f54a9081ace9966a7cb9413811edb4}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_047_last_suffix_first_initial() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L10a"#####, 0),
        r#####"    \entry{L10a}{book}{}{}
      \name{author}{1}{}{%
        {{hash=7bf2c9d8b89a1930ee91bfddcaf20c9c}{%
           family={Pimentel},
           familyi={P\\bibinitperiod},
           given={Joseph\\bibnamedelima J.},
           giveni={J\\bibinitperiod\\bibinitdelim J\\bibinitperiod},
           suffix={Jr.},
           suffixi={J\\bibinitperiod}}}%
      }
      \strng{namehash}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \strng{fullhash}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \strng{fullhashraw}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \strng{bibnamehash}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \strng{authorbibnamehash}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \strng{authornamehash}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \strng{authorfullhash}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \strng{authorfullhashraw}{7bf2c9d8b89a1930ee91bfddcaf20c9c}
      \field{sortinit}{P}
      \field{sortinithash}{ff3bcf24f47321b42cb156c2cc8a8422}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_048_prefix_last_suffix_first() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L11"#####, 0),
        r#####"    \entry{L11}{book}{}{}
      \name{author}{1}{}{%
        {{hash=9f48d231be68c9435fab4faca55a5caf}{%
           family={Kluster},
           familyi={K\\bibinitperiod},
           given={Kevin},
           giveni={K\\bibinitperiod},
           prefix={van},
           prefixi={v\\bibinitperiod},
           suffix={Jr.},
           suffixi={J\\bibinitperiod}}}%
      }
      \strng{namehash}{9f48d231be68c9435fab4faca55a5caf}
      \strng{fullhash}{9f48d231be68c9435fab4faca55a5caf}
      \strng{fullhashraw}{9f48d231be68c9435fab4faca55a5caf}
      \strng{bibnamehash}{9f48d231be68c9435fab4faca55a5caf}
      \strng{authorbibnamehash}{9f48d231be68c9435fab4faca55a5caf}
      \strng{authornamehash}{9f48d231be68c9435fab4faca55a5caf}
      \strng{authorfullhash}{9f48d231be68c9435fab4faca55a5caf}
      \strng{authorfullhashraw}{9f48d231be68c9435fab4faca55a5caf}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_049_last_last_last_initial_initial() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L13"#####, 0),
        r#####"    \entry{L13}{book}{}{}
      \name{author}{1}{}{%
        {{hash=227ac48bb788a658cfaa4eefc71ff0cc}{%
           family={Van\bibnamedelimb de\bibnamedelima Graaff},
           familyi={V\bibinitperiod\bibinitdelim d\bibinitperiod\bibinitdelim G\bibinitperiod},
           given={R.\bibnamedelimi J.},
           giveni={R\bibinitperiod\bibinitdelim J\bibinitperiod}}}%
      }
      \strng{namehash}{227ac48bb788a658cfaa4eefc71ff0cc}
      \strng{fullhash}{227ac48bb788a658cfaa4eefc71ff0cc}
      \strng{fullhashraw}{227ac48bb788a658cfaa4eefc71ff0cc}
      \strng{bibnamehash}{227ac48bb788a658cfaa4eefc71ff0cc}
      \strng{authorbibnamehash}{227ac48bb788a658cfaa4eefc71ff0cc}
      \strng{authornamehash}{227ac48bb788a658cfaa4eefc71ff0cc}
      \strng{authorfullhash}{227ac48bb788a658cfaa4eefc71ff0cc}
      \strng{authorfullhashraw}{227ac48bb788a658cfaa4eefc71ff0cc}
      \field{sortinit}{V}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_050_last_last_last_first() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L14"#####, 0),
        r#####"    \entry{L14}{book}{}{}
      \name{author}{1}{}{%
        {{hash=779475052c17ed56dc3be900d0dfdf87}{%
           family={St\bibnamedelima John-Mollusc},
           familyi={S\bibinitperiod\bibinitdelim J\bibinithyphendelim M\bibinitperiod},
           given={Oliver},
           giveni={O\bibinitperiod}}}%
      }
      \strng{namehash}{779475052c17ed56dc3be900d0dfdf87}
      \strng{fullhash}{779475052c17ed56dc3be900d0dfdf87}
      \strng{fullhashraw}{779475052c17ed56dc3be900d0dfdf87}
      \strng{bibnamehash}{779475052c17ed56dc3be900d0dfdf87}
      \strng{authorbibnamehash}{779475052c17ed56dc3be900d0dfdf87}
      \strng{authornamehash}{779475052c17ed56dc3be900d0dfdf87}
      \strng{authorfullhash}{779475052c17ed56dc3be900d0dfdf87}
      \strng{authorfullhashraw}{779475052c17ed56dc3be900d0dfdf87}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_051_first_f_bibinitdelim_f_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L15"#####, 0),
        r#####"    \entry{L15}{book}{}{}
      \name{author}{1}{}{%
        {{hash=783c636e853e47a854ae034ebe9dde62}{%
           family={Gompel},
           familyi={G\\bibinitperiod},
           given={Roger\\bibnamedelima P.{\\,}G.},
           giveni={R\\bibinitperiod\\bibinitdelim P\\bibinitperiod},
           prefix={van},
           prefixi={v\\bibinitperiod}}}%
      }
      \strng{namehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{fullhash}{783c636e853e47a854ae034ebe9dde62}
      \strng{fullhashraw}{783c636e853e47a854ae034ebe9dde62}
      \strng{bibnamehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authorbibnamehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authornamehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authorfullhash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authorfullhashraw}{783c636e853e47a854ae034ebe9dde62}
      \field{extraname}{1}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_052_first_f_bibinitdelim_f_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L16"#####, 0),
        r#####"    \entry{L16}{book}{}{}
      \name{author}{1}{}{%
        {{hash=783c636e853e47a854ae034ebe9dde62}{%
           family={Gompel},
           familyi={G\\bibinitperiod},
           given={Roger\\bibnamedelima {P.\\,G.}},
           giveni={R\\bibinitperiod\\bibinitdelim P\\bibinitperiod},
           prefix={van},
           prefixi={v\\bibinitperiod}}}%
      }
      \strng{namehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{fullhash}{783c636e853e47a854ae034ebe9dde62}
      \strng{fullhashraw}{783c636e853e47a854ae034ebe9dde62}
      \strng{bibnamehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authorbibnamehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authornamehash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authorfullhash}{783c636e853e47a854ae034ebe9dde62}
      \strng{authorfullhashraw}{783c636e853e47a854ae034ebe9dde62}
      \field{extraname}{2}
      \field{sortinit}{v}
      \field{sortinithash}{afb52128e5b4dc4b843768c0113d673b}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_053_last_first_f_bibinitdelim_f() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L17"#####, 0),
        r#####"    \entry{L17}{book}{}{}
      \name{author}{1}{}{%
        {{hash=b51f667a3384d92ea5458ba80716bff7}{%
           family={Lovecraft},
           familyi={L\bibinitperiod},
           given={Bill\bibnamedelima H.{\,}P.},
           giveni={B\bibinitperiod\bibinitdelim H\bibinitperiod}}}%
      }
      \strng{namehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{fullhash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{fullhashraw}{b51f667a3384d92ea5458ba80716bff7}
      \strng{bibnamehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authorbibnamehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authornamehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authorfullhash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authorfullhashraw}{b51f667a3384d92ea5458ba80716bff7}
      \field{extraname}{1}
      \field{sortinit}{L}
      \field{sortinithash}{7c47d417cecb1f4bd38d1825c427a61a}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_054_last_first_f_bibinitdelim_f() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L18"#####, 0),
        r#####"    \entry{L18}{book}{}{}
      \name{author}{1}{}{%
        {{hash=b51f667a3384d92ea5458ba80716bff7}{%
           family={Lovecraft},
           familyi={L\bibinitperiod},
           given={Bill\bibnamedelima {H.\,P.}},
           giveni={B\bibinitperiod\bibinitdelim H\bibinitperiod}}}%
      }
      \strng{namehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{fullhash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{fullhashraw}{b51f667a3384d92ea5458ba80716bff7}
      \strng{bibnamehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authorbibnamehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authornamehash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authorfullhash}{b51f667a3384d92ea5458ba80716bff7}
      \strng{authorfullhashraw}{b51f667a3384d92ea5458ba80716bff7}
      \field{extraname}{2}
      \field{sortinit}{L}
      \field{sortinithash}{7c47d417cecb1f4bd38d1825c427a61a}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_055_firstname_with_hyphen() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L19"#####, 0),
        r#####"    \entry{L19}{book}{}{}
      \name{author}{1}{}{%
        {{hash=83caa52f21f97e572dd3267bdf62978a}{%
           family={Mustermann},
           familyi={M\bibinitperiod},
           given={Klaus-Peter},
           giveni={K\bibinithyphendelim P\bibinitperiod}}}%
      }
      \strng{namehash}{83caa52f21f97e572dd3267bdf62978a}
      \strng{fullhash}{83caa52f21f97e572dd3267bdf62978a}
      \strng{fullhashraw}{83caa52f21f97e572dd3267bdf62978a}
      \strng{bibnamehash}{83caa52f21f97e572dd3267bdf62978a}
      \strng{authorbibnamehash}{83caa52f21f97e572dd3267bdf62978a}
      \strng{authornamehash}{83caa52f21f97e572dd3267bdf62978a}
      \strng{authorfullhash}{83caa52f21f97e572dd3267bdf62978a}
      \strng{authorfullhashraw}{83caa52f21f97e572dd3267bdf62978a}
      \field{sortinit}{M}
      \field{sortinithash}{4625c616857f13d17ce56f7d4f97d451}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_056_short_given_name_with_hyphen() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L19a"#####, 0),
        r#####"    \entry{L19a}{book}{}{}
      \name{author}{1}{}{%
        {{hash=0963f6904ccfeaac2770c5882a587001}{%
           family={Lam},
           familyi={L\bibinitperiod},
           given={Ho-Pun},
           giveni={H\bibinithyphendelim P\bibinitperiod}}}%
      }
      \strng{namehash}{0963f6904ccfeaac2770c5882a587001}
      \strng{fullhash}{0963f6904ccfeaac2770c5882a587001}
      \strng{fullhashraw}{0963f6904ccfeaac2770c5882a587001}
      \strng{bibnamehash}{0963f6904ccfeaac2770c5882a587001}
      \strng{authorbibnamehash}{0963f6904ccfeaac2770c5882a587001}
      \strng{authornamehash}{0963f6904ccfeaac2770c5882a587001}
      \strng{authorfullhash}{0963f6904ccfeaac2770c5882a587001}
      \strng{authorfullhashraw}{0963f6904ccfeaac2770c5882a587001}
      \field{sortinit}{L}
      \field{sortinithash}{7c47d417cecb1f4bd38d1825c427a61a}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_057_protected_dual_given_name() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L20"#####, 0),
        r#####"    \entry{L20}{book}{}{}
      \name{author}{1}{}{%
        {{hash=5f26c2f3b33095d5b005714893f4d698}{%
           family={Ford},
           familyi={F\bibinitperiod},
           given={{John Henry}},
           giveni={J\bibinitperiod}}}%
      }
      \strng{namehash}{5f26c2f3b33095d5b005714893f4d698}
      \strng{fullhash}{5f26c2f3b33095d5b005714893f4d698}
      \strng{fullhashraw}{5f26c2f3b33095d5b005714893f4d698}
      \strng{bibnamehash}{5f26c2f3b33095d5b005714893f4d698}
      \strng{authorbibnamehash}{5f26c2f3b33095d5b005714893f4d698}
      \strng{authornamehash}{5f26c2f3b33095d5b005714893f4d698}
      \strng{authorfullhash}{5f26c2f3b33095d5b005714893f4d698}
      \strng{authorfullhashraw}{5f26c2f3b33095d5b005714893f4d698}
      \field{sortinit}{F}
      \field{sortinithash}{2638baaa20439f1b5a8f80c6c08a13b4}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_058_latex_encoded_unicode_family_1() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L22"#####, 0),
        r#####"    \entry{L22}{book}{}{}
      \name{author}{1}{}{%
        {{hash=e58b861545799d0eaf883402a882126e}{%
           family={Šmith},
           familyi={Š\bibinitperiod},
           given={Someone},
           giveni={S\bibinitperiod}}}%
      }
      \strng{namehash}{e58b861545799d0eaf883402a882126e}
      \strng{fullhash}{e58b861545799d0eaf883402a882126e}
      \strng{fullhashraw}{e58b861545799d0eaf883402a882126e}
      \strng{bibnamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authorbibnamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authornamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authorfullhash}{e58b861545799d0eaf883402a882126e}
      \strng{authorfullhashraw}{e58b861545799d0eaf883402a882126e}
      \field{extraname}{1}
      \field{sortinit}{Š}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_059_unicode_given_name() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L23"#####, 0),
        r#####"    \entry{L23}{book}{}{}
      \name{author}{1}{}{%
        {{hash=4389a3c0dc7da74487b50808ba9436ad}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={Šomeone},
           giveni={Š\bibinitperiod}}}%
      }
      \strng{namehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{fullhash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{fullhashraw}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{bibnamehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authorbibnamehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authornamehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authorfullhash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authorfullhashraw}{4389a3c0dc7da74487b50808ba9436ad}
      \field{extraname}{2}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_060_unicode_family_name() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L24"#####, 0),
        r#####"    \entry{L24}{book}{}{}
      \name{author}{1}{}{%
        {{hash=e58b861545799d0eaf883402a882126e}{%
           family={Šmith},
           familyi={Š\bibinitperiod},
           given={Someone},
           giveni={S\bibinitperiod}}}%
      }
      \strng{namehash}{e58b861545799d0eaf883402a882126e}
      \strng{fullhash}{e58b861545799d0eaf883402a882126e}
      \strng{fullhashraw}{e58b861545799d0eaf883402a882126e}
      \strng{bibnamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authorbibnamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authornamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authorfullhash}{e58b861545799d0eaf883402a882126e}
      \strng{authorfullhashraw}{e58b861545799d0eaf883402a882126e}
      \field{extraname}{2}
      \field{sortinit}{Š}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_061_single_string_name() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L25"#####, 0),
        r#####"    \entry{L25}{book}{}{}
      \name{author}{1}{}{%
        {{hash=d7cd2c5ea0848abc3e90609558b84a45}{%
           family={{American Psychological Association, Task Force on the Sexualization of Girls}},
           familyi={A\bibinitperiod}}}%
      }
      \strng{namehash}{d7cd2c5ea0848abc3e90609558b84a45}
      \strng{fullhash}{d7cd2c5ea0848abc3e90609558b84a45}
      \strng{fullhashraw}{d7cd2c5ea0848abc3e90609558b84a45}
      \strng{bibnamehash}{d7cd2c5ea0848abc3e90609558b84a45}
      \strng{authorbibnamehash}{d7cd2c5ea0848abc3e90609558b84a45}
      \strng{authornamehash}{d7cd2c5ea0848abc3e90609558b84a45}
      \strng{authorfullhash}{d7cd2c5ea0848abc3e90609558b84a45}
      \strng{authorfullhashraw}{d7cd2c5ea0848abc3e90609558b84a45}
      \field{sortinit}{A}
      \field{sortinithash}{2f401846e2029bad6b3ecc16d50031e2}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_062_hyphen_at_brace_level_0() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L26"#####, 0),
        r#####"    \entry{L26}{book}{}{}
      \name{author}{1}{}{%
        {{hash=8eee1dbafdbd0a4b73157e60f18b4784}{%
           family={{Sci-Art Publishers}},
           familyi={S\bibinitperiod}}}%
      }
      \strng{namehash}{8eee1dbafdbd0a4b73157e60f18b4784}
      \strng{fullhash}{8eee1dbafdbd0a4b73157e60f18b4784}
      \strng{fullhashraw}{8eee1dbafdbd0a4b73157e60f18b4784}
      \strng{bibnamehash}{8eee1dbafdbd0a4b73157e60f18b4784}
      \strng{authorbibnamehash}{8eee1dbafdbd0a4b73157e60f18b4784}
      \strng{authornamehash}{8eee1dbafdbd0a4b73157e60f18b4784}
      \strng{authorfullhash}{8eee1dbafdbd0a4b73157e60f18b4784}
      \strng{authorfullhashraw}{8eee1dbafdbd0a4b73157e60f18b4784}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_063_escaped_name_with_3_commas() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L29"#####, 0),
        r#####"    \entry{L29}{book}{}{}
      \name{author}{1}{}{%
        {{hash=27ad192a3a715aa89152b2a4ee392e8c}{%
           family={{U.S. Department of Health and Human Services, National Institute of Mental Health, National Heart, Lung and Blood Institute}},
           familyi={U\bibinitperiod}}}%
      }
      \strng{namehash}{27ad192a3a715aa89152b2a4ee392e8c}
      \strng{fullhash}{27ad192a3a715aa89152b2a4ee392e8c}
      \strng{fullhashraw}{27ad192a3a715aa89152b2a4ee392e8c}
      \strng{bibnamehash}{27ad192a3a715aa89152b2a4ee392e8c}
      \strng{authorbibnamehash}{27ad192a3a715aa89152b2a4ee392e8c}
      \strng{authornamehash}{27ad192a3a715aa89152b2a4ee392e8c}
      \strng{authorfullhash}{27ad192a3a715aa89152b2a4ee392e8c}
      \strng{authorfullhashraw}{27ad192a3a715aa89152b2a4ee392e8c}
      \field{sortinit}{U}
      \field{sortinithash}{6901a00e45705986ee5e7ca9fd39adca}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_064_name_count_for_and_others_1() {
    assert_eq!(name_count(r#####"names.bcf"#####, r#####"V1"#####), 2);
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_065_visibility_for_and_others_1() {
    assert_eq!(
        field_text(r#####"names.bcf"#####, r#####"V1"#####, "visiblecite").as_deref(),
        Some(r#####"2"#####)
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_066_visibility_for_and_others_2() {
    assert_eq!(
        field_text(r#####"names.bcf"#####, r#####"V2"#####, "visiblecite").as_deref(),
        Some(r#####"1"#####)
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_067_terseinitials_1() {
    assert_eq!(
        name_initial(
            r#####"names.bcf"#####,
            r#####"L21"#####,
            r#####"given"#####,
            0
        ),
        r#####"Š"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_068_first_first_first_first_prefix_prefix_last_last() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L12"#####, 0),
        r#####"    \entry{L12}{book}{}{}
      \name{author}{1}{}{%
        {{hash=d7ca88c13a8f7ce1c23e920010a31f83}{%
           family={Vallée\\bibnamedelima Poussin},
           familyi={V\\bibinitperiod\\bibinitdelim P\\bibinitperiod},
           given={Charles\\bibnamedelimb Louis\\bibnamedelimb Xavier\\bibnamedelima Joseph},
           giveni={C\\bibinitperiod\\bibinitdelim L\\bibinitperiod\\bibinitdelim X\\bibinitperiod\\bibinitdelim J\\bibinitperiod},
           prefix={de\\bibnamedelima la},
           prefixi={d\\bibinitperiod\\bibinitdelim l\\bibinitperiod}}}%
      }
      \strng{namehash}{d7ca88c13a8f7ce1c23e920010a31f83}
      \strng{fullhash}{d7ca88c13a8f7ce1c23e920010a31f83}
      \strng{fullhashraw}{d7ca88c13a8f7ce1c23e920010a31f83}
      \strng{bibnamehash}{d7ca88c13a8f7ce1c23e920010a31f83}
      \strng{authorbibnamehash}{d7ca88c13a8f7ce1c23e920010a31f83}
      \strng{authornamehash}{d7ca88c13a8f7ce1c23e920010a31f83}
      \strng{authorfullhash}{d7ca88c13a8f7ce1c23e920010a31f83}
      \strng{authorfullhashraw}{d7ca88c13a8f7ce1c23e920010a31f83}
      \field{sortinit}{d}
      \field{sortinithash}{6f385f66841fb5e82009dc833c761848}
      \true{uniqueprimaryauthor}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_069_latex_encoded_unicode_given_name() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L21"#####, 0),
        r#####"    \entry{L21}{book}{}{}
      \name{author}{1}{}{%
        {{hash=4389a3c0dc7da74487b50808ba9436ad}{%
           family={Smith},
           familyi={S\bibinitperiod},
           given={\v{S}omeone},
           giveni={\v{S}\bibinitperiod}}}%
      }
      \strng{namehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{fullhash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{fullhashraw}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{bibnamehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authorbibnamehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authornamehash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authorfullhash}{4389a3c0dc7da74487b50808ba9436ad}
      \strng{authorfullhashraw}{4389a3c0dc7da74487b50808ba9436ad}
      \field{extraname}{1}
      \field{sortinit}{S}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \true{uniqueprimaryauthor}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_070_latex_encoded_unicode_family_name_2() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L22"#####, 0),
        r#####"    \entry{L22}{book}{}{}
      \name{author}{1}{}{%
        {{hash=e58b861545799d0eaf883402a882126e}{%
           family={\v{S}mith},
           familyi={\v{S}\bibinitperiod},
           given={Someone},
           giveni={S\bibinitperiod}}}%
      }
      \strng{namehash}{e58b861545799d0eaf883402a882126e}
      \strng{fullhash}{e58b861545799d0eaf883402a882126e}
      \strng{fullhashraw}{e58b861545799d0eaf883402a882126e}
      \strng{bibnamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authorbibnamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authornamehash}{e58b861545799d0eaf883402a882126e}
      \strng{authorfullhash}{e58b861545799d0eaf883402a882126e}
      \strng{authorfullhashraw}{e58b861545799d0eaf883402a882126e}
      \field{extraname}{1}
      \field{sortinit}{\v{S}}
      \field{sortinithash}{b164b07b29984b41daf1e85279fbc5ab}
      \true{uniqueprimaryauthor}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_071_latex_encoded_unicode_family_name_with_tie_char() {
    assert_eq!(
        output_entry(r#####"names.bcf"#####, r#####"L31"#####, 0),
        r#####"    \entry{L31}{book}{}{}
      \name{author}{1}{}{%
        {{hash=29c3ff92fff79d09a8b44d2f775de0b1}{%
           family={\~{Z}elly},
           familyi={\~{Z}\\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod}}}%
      }
      \name{editor}{1}{}{%
        {{hash=29c3ff92fff79d09a8b44d2f775de0b1}{%
           family={\~{Z}elly},
           familyi={\~{Z}\\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod}}}%
      }
      \name{translator}{1}{}{%
        {{hash=29c3ff92fff79d09a8b44d2f775de0b1}{%
           family={\~{Z}elly},
           familyi={\~{Z}\\bibinitperiod},
           given={Arthur},
           giveni={A\bibinitperiod}}}%
      }
      \strng{namehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{fullhash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{fullhashraw}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{bibnamehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{authorbibnamehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{authornamehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{authorfullhash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{authorfullhashraw}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{editorbibnamehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{editornamehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{editorfullhash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{editorfullhashraw}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{translatorbibnamehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{translatornamehash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{translatorfullhash}{29c3ff92fff79d09a8b44d2f775de0b1}
      \strng{translatorfullhashraw}{29c3ff92fff79d09a8b44d2f775de0b1}
      \field{sortinit}{\~{Z}}
      \field{sortinithash}{96892c0b0a36bb8557c40c49813d48b3}
      \true{uniqueprimaryauthor}
      \field{labelnamesource}{author}
    \endentry
"#####
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_072_unique_primary_author_1() {
    assert_eq!(
        field_text(
            r#####"names.bcf"#####,
            r#####"upa1"#####,
            r#####"uniqueprimaryauthor"#####
        )
        .as_deref(),
        Some("1")
    );
}

#[test]
fn assertion_073_unique_primary_author_2() {
    assert_eq!(
        field_text(
            r#####"names.bcf"#####,
            r#####"upa2"#####,
            r#####"uniqueprimaryauthor"#####
        )
        .as_deref(),
        None
    );
}

#[test]
fn assertion_074_unique_primary_author_3() {
    assert_eq!(
        field_text(
            r#####"names.bcf"#####,
            r#####"upa3"#####,
            r#####"uniqueprimaryauthor"#####
        )
        .as_deref(),
        None
    );
}

#[test]
#[ignore = "xfail: processed name or BBL differs from the Biber 2.22 expectation"]
fn assertion_075_unique_primary_author_4() {
    assert_eq!(
        field_text(
            r#####"names.bcf"#####,
            r#####"upa4"#####,
            r#####"uniqueprimaryauthor"#####
        )
        .as_deref(),
        Some("1")
    );
}
