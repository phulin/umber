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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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

#[allow(dead_code)]
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_001_parsename_x_1() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=John,family=Doe"#####
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
                value: None,
                initials: vec![]
            }
        }
    );
}

#[test]
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_002_parsename_x_2() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"family=Doe, suffix=Jr, given=John, given-i=J"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_003_parsename_x_3() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"prefix=von, family=Berlichingen zu Hornberg, given=Johann Gottfried"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_004_parsename_x_4() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"prefix=von, family=Berlichingen zu Hornberg, given=Johann Gottfried"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_005_parsename_x_5() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####""family={Robert and Sons, Inc.}""#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_006_parsename_x_6() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"family=al-Ṣāliḥ, given=ʿAbdallāh"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_007_parsename_x_7() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=Jean Charles Gabriel, prefix=de la, family=Vallée Poussin"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_008_parsename_x_8() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given={Jean Charles Gabriel}, prefix=de la, family=Vallée Poussin"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_009_parsename_x_9() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=Jean Charles Gabriel de la Vallée, given-i=JCGdV, family=Poussin"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Jean Charles Gabriel de la~Vallée"#####.to_owned()),
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_010_parsename_x_10() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=Jean Charles Gabriel, prefix=de la, family={Vallée Poussin}"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_011_parsename_x_11() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given={Jean Charles Gabriel}, prefix=de la, family={Vallée Poussin}"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_012_parsename_x_12() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=Jean Charles Gabriel, family=Poussin"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_013_parsename_x_13() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=Jean Charles, family={Poussin Lecoq}"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_014_parsename_x_14() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=J. C. G., prefix=de la, family=Vallée Poussin"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_015_parsename_x_15() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=E. S., family=El-Mallah"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"E.~S."#####.to_owned()),
                initials: vec![r#####"E"#####.to_owned(), r#####"S"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"El-Mallah"#####.to_owned()),
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_016_parsename_x_16() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"given=E. S., family=Kent-Boswell"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"E.~S."#####.to_owned()),
                initials: vec![r#####"E"#####.to_owned(), r#####"S"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Kent-Boswell"#####.to_owned()),
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_017_parsename_x_17() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"family=Other, given=A.~N."#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_018_parsename_x_18() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"family={British National Corpus}"#####
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_019_parsename_x_19() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"sortingnamekeytemplatename=test, family=Smith, given=Bill"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: Some(r#####"Bill"#####.to_owned()),
                initials: vec![r#####"B"#####.to_owned()]
            },
            family: PartSnapshot {
                value: Some(r#####"Smith"#####.to_owned()),
                initials: vec![r#####"S"#####.to_owned()]
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
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_020_parsename_x_19a() {
    assert_eq!(
        parsed_name(
            r#####"names_x.bcf"#####,
            r#####"sortingnamekeytemplatename=test, family=Smith, given=Bill"#####
        )
        .sorting_name_key_template(),
        Some("test")
    );
}

#[test]
#[ignore = "xfail: bib-engine does not yet parse Biber extended-name records"]
fn assertion_021_parsename_x_20() {
    assert_eq!(
        name_snapshot(&parsed_name(
            r#####"names_x.bcf"#####,
            r#####"family=Doe, family-i={Do}"#####
        )),
        NameSnapshot {
            given: PartSnapshot {
                value: None,
                initials: vec![]
            },
            family: PartSnapshot {
                value: Some(r#####"Doe"#####.to_owned()),
                initials: vec![r#####"Do"#####.to_owned()]
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
