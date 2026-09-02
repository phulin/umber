use bib_engine::{
    BibAttempt, BibJob, BibOptionsBuilder, BibResult, BibSession, BibSessionOptions, EntryId,
    FieldId, FieldValue, OutputFormat, OutputRequest, ProjectWorkspace, ResolvedFile, SectionId,
    VfsLimits, VirtualPath,
};

pub(super) const PINNED_COMMIT: &str = "74252e608e5f8115375c532eb25416430a9f52eb";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct CompatibilityCase {
    pub upstream_commit: &'static str,
    pub module: &'static str,
    pub order: usize,
    pub name: &'static str,
    pub xfail: Option<&'static str>,
    pub input: FixtureInput,
    pub output: OutputExpectation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct FixtureInput {
    pub control: &'static str,
    pub options: &'static [(&'static str, &'static str)],
    pub requests: &'static [OutputFixture],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct OutputFixture {
    pub path: &'static str,
    pub format: OutputFormat,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum OutputExpectation {
    Field {
        entry: &'static str,
        field: &'static str,
        expected: Option<&'static str>,
    },
    NameAssignment {
        entry: &'static str,
        name_index: usize,
        assignment: &'static str,
        expected: Option<&'static str>,
    },
    BblEntry {
        entry: &'static str,
        expected: &'static str,
    },
}

pub(super) const BBL_OUTPUT: &[OutputFixture] = &[OutputFixture {
    path: "native.bbl",
    format: OutputFormat::Bbl,
}];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ModuleManifest {
    name: &'static str,
    assertions: usize,
    generated: Option<&'static [CompatibilityCase]>,
}

const MODULES: &[ModuleManifest] = &[
    module("annotations", 2),
    module("basic_misc", 72),
    module("bcfvalidation", 53),
    module("biblatexml", 5),
    module("bibtex_aliases", 25),
    module("bibtex_output", 6),
    module("configfile", 10),
    module("crossrefs", 37),
    module("datalists", 15),
    module("dateformats", 56),
    module("dm_constraints", 16),
    module("encoding", 10),
    generated("extradate", super::extradate::CASES),
    generated("extratitle", super::extratitle::CASES),
    generated("extratitleyear", super::extratitleyear::CASES),
    module("full_bbl", 5),
    module("full_bibtex", 2),
    module("full_dot", 2),
    generated("labelalpha", super::labelalpha::CASES),
    generated("labelalphaname", super::labelalphaname::CASES),
    module("labelname", 4),
    module("langtags", 19),
    module("maps", 8),
    module("names", 75),
    module("names_x", 21),
    module("options", 9),
    module("related_entries", 15),
    module("remote_files", 1),
    module("sections", 14),
    module("sections_complex", 68),
    module("set_dynamic", 7),
    module("set_legacy", 3),
    module("set_static", 6),
    module("skips", 15),
    module("skipsg", 3),
    module("sort_case", 2),
    module("sort_complex", 9),
    module("sort_names", 1),
    module("sort_order", 18),
    module("sort_uc", 6),
    module("sorting", 52),
    module("tool", 19),
    module("tool_bltxml", 3),
    module("tool_bltxml_inout", 1),
    module("tool_config", 12),
    module("translit", 1),
    module("truncation", 12),
    generated("uniqueness", super::uniqueness::CASES),
    module("uniqueness_nameparts", 36),
    module("utils", 89),
    module("xdata", 13),
];

const fn module(name: &'static str, assertions: usize) -> ModuleManifest {
    ModuleManifest {
        name,
        assertions,
        generated: None,
    }
}

const fn generated(name: &'static str, cases: &'static [CompatibilityCase]) -> ModuleManifest {
    ModuleManifest {
        name,
        assertions: cases.len(),
        generated: Some(cases),
    }
}

pub(crate) fn assert_manifest_complete() {
    assert_eq!(MODULES.len(), 51, "one typed row per upstream module");
    assert_eq!(
        MODULES
            .iter()
            .map(|module| module.assertions)
            .sum::<usize>(),
        1_275,
        "typed module rows own the complete upstream assertion count"
    );
    for (module_index, module) in MODULES.iter().enumerate() {
        assert!(module.assertions > 0, "empty module {}", module.name);
        assert!(
            MODULES[..module_index]
                .iter()
                .all(|earlier| earlier.name != module.name),
            "duplicate module {}",
            module.name
        );
        let Some(cases) = module.generated else {
            continue;
        };
        assert_eq!(cases.len(), module.assertions);
        for (index, case) in cases.iter().enumerate() {
            assert_eq!(case.upstream_commit, PINNED_COMMIT);
            assert_eq!(case.module, module.name);
            assert_eq!(case.order, index + 1, "order for {}", case.name);
            assert!(
                case.name
                    .starts_with(&format!("assertion_{:03}_", case.order)),
                "name/order mismatch for {}",
                case.name
            );
            assert!(
                case.xfail
                    .is_none_or(|reason| reason.starts_with("xfail: ") && reason.len() > 7),
                "invalid xfail for {}",
                case.name
            );
            assert!(case.input.control.ends_with(".bcf"));
            assert_eq!(case.input.requests, BBL_OUTPUT);
            for (option_index, &(key, _)) in case.input.options.iter().enumerate() {
                assert!(
                    case.input.options[..option_index]
                        .iter()
                        .all(|&(earlier, _)| earlier != key),
                    "duplicate option {key} in {}",
                    case.name
                );
            }
        }
    }
}

pub(super) fn run(case: &CompatibilityCase) {
    assert_eq!(case.upstream_commit, PINNED_COMMIT);
    let fixture_dir = test_support::repository_root().join("tests/corpus/bib/upstream-2.22/tdata");
    let control = VirtualPath::user(case.input.control).expect("valid control path");
    let mut control_bytes =
        String::from_utf8(crate::fixtures::read(fixture_dir.join(case.input.control)))
            .expect("BCF is UTF-8");
    for &(key, value) in case.input.options {
        override_scalar_option(&mut control_bytes, key, value);
    }

    let mut workspace = ProjectWorkspace::new(VfsLimits::default()).expect("valid VFS limits");
    workspace
        .register_user(control.clone(), control_bytes.into_bytes())
        .expect("unique control file");
    let mut options = BibOptionsBuilder::new();
    for request in case.input.requests {
        options
            .output(OutputRequest::new(
                VirtualPath::user(request.path).expect("valid output path"),
                request.format,
            ))
            .expect("unique output");
    }
    let job = BibJob::new(control, options.freeze());

    let mut cached = BibSession::default();
    let cached_result = complete(&mut cached, &job, &mut workspace, &fixture_dir);
    let mut cold = BibSession::new(BibSessionOptions::default().without_caches())
        .expect("valid cache-free session");
    let cold_result = match cold.process(&job, &workspace.snapshot()) {
        BibAttempt::Complete(result) => result,
        other => panic!("fully provisioned cache-free replay did not complete: {other:?}"),
    };
    assert_eq!(cached_result, cold_result, "cache purity for {}", case.name);

    match case.output {
        OutputExpectation::Field {
            entry,
            field,
            expected,
        } => assert_eq!(
            field_text(&cached_result, entry, field).as_deref(),
            expected
        ),
        OutputExpectation::NameAssignment {
            entry,
            name_index,
            assignment,
            expected,
        } => assert_eq!(
            name_assignment(&cached_result, entry, name_index, assignment).as_deref(),
            expected
        ),
        OutputExpectation::BblEntry { entry, expected } => {
            assert_eq!(output_entry(&cached_result, entry), expected)
        }
    }
}

fn complete(
    session: &mut BibSession,
    job: &BibJob,
    workspace: &mut ProjectWorkspace,
    fixture_dir: &std::path::Path,
) -> BibResult {
    loop {
        match session.process(job, &workspace.snapshot()) {
            BibAttempt::Complete(result) => return result,
            BibAttempt::NeedResources(requests) => {
                workspace.expect(&requests);
                for request in requests
                    .required
                    .iter()
                    .chain(requests.prefetch_hints.iter())
                {
                    if let Some(bytes) =
                        crate::fixtures::read_optional(fixture_dir.join(request.key().name()))
                    {
                        workspace
                            .provision(ResolvedFile {
                                request: request.key().clone(),
                                virtual_path: format!("/texlive/bib/{}", request.key().name()),
                                bytes: bytes.into(),
                                expected_digest: None,
                            })
                            .expect("requested fixture is valid");
                    }
                }
            }
            BibAttempt::Failed(failure) => panic!("fixture processing failed: {failure:?}"),
        }
    }
}

fn override_scalar_option(control: &mut String, key: &str, value: &str) {
    if key == "extradatespec" {
        let start_tag = "<bcf:extradatespec>";
        let end_tag = "</bcf:extradatespec>";
        let start = control.find(start_tag).expect("extradate spec exists");
        let end = control[start..]
            .find(end_tag)
            .map(|offset| start + offset + end_tag.len())
            .expect("extradate spec is terminated");
        let scopes = value
            .split(';')
            .map(|scope| {
                let fields = scope
                    .split(',')
                    .enumerate()
                    .map(|(index, field)| {
                        format!(
                            "      <bcf:field order=\"{}\">{field}</bcf:field>\n",
                            index + 1
                        )
                    })
                    .collect::<String>();
                format!("    <bcf:scope>\n{fields}    </bcf:scope>\n")
            })
            .collect::<String>();
        control.replace_range(start..end, &format!("{start_tag}\n{scopes}  {end_tag}"));
        return;
    }
    let key_tag = format!("<bcf:key>{key}</bcf:key>");
    let key_at = control
        .find(&key_tag)
        .expect("option exists in committed BCF");
    let value_start = control[key_at..]
        .find("<bcf:value>")
        .map(|offset| key_at + offset + "<bcf:value>".len())
        .expect("option has a value");
    let value_end = control[value_start..]
        .find("</bcf:value>")
        .map(|offset| value_start + offset)
        .expect("option value is terminated");
    control.replace_range(value_start..value_end, value);
}

fn field_text(result: &BibResult, entry_key: &str, field_name: &str) -> Option<String> {
    let entry = result
        .document()
        .section(SectionId::new(0))?
        .entry(&EntryId::new(entry_key).expect("valid entry key"))?;
    match entry
        .fields()
        .get(&FieldId::new(field_name).expect("valid field name"))?
    {
        FieldValue::Literal(value) => Some(value.as_str().to_owned()),
        FieldValue::Verbatim(value) => Some(value.as_str().to_owned()),
        FieldValue::Integer(value) => Some(value.to_string()),
        FieldValue::Boolean(value) => Some(if *value { "1" } else { "0" }.to_owned()),
        _ => None,
    }
}

fn name_assignment(
    result: &BibResult,
    entry_key: &str,
    name_index: usize,
    assignment_key: &str,
) -> Option<String> {
    let entry = result
        .document()
        .section(SectionId::new(0))?
        .entry(&EntryId::new(entry_key).expect("valid entry key"))?;
    let source = match entry
        .fields()
        .get(&FieldId::new("labelnamesource").expect("valid field name"))?
    {
        FieldValue::Literal(value) => value.as_str(),
        _ => return None,
    };
    let names = match entry
        .fields()
        .get(&FieldId::new(source).expect("valid name-list field"))?
    {
        FieldValue::NameList(names) => names,
        _ => return None,
    };
    names
        .iter()
        .nth(name_index.checked_sub(1)?)?
        .assignments()
        .find(|assignment| assignment.key() == assignment_key)
        .map(|assignment| assignment.value().to_owned())
}

fn output_entry(result: &BibResult, entry_key: &str) -> String {
    let bbl = result
        .files()
        .find(|file| file.path().as_str().ends_with("native.bbl"))
        .map(|file| String::from_utf8_lossy(file.bytes()).into_owned())
        .unwrap_or_default();
    let marker = format!("\\\\entry{{{entry_key}}}");
    let marker_at = bbl
        .find(&marker)
        .expect("entry is present in generated BBL");
    let start = bbl[..marker_at].rfind("    ").unwrap_or(marker_at);
    let end = bbl[marker_at..]
        .find("\\\\endentry")
        .map(|offset| marker_at + offset + "\\\\endentry".len())
        .expect("entry is terminated");
    bbl[start..end].to_owned()
}

macro_rules! compatibility_cases {
    (
        module $module:literal;
        $(
            $(#[ignore = $xfail:literal])?
            $order:literal $name:ident {
                control: $control:literal,
                options: $options:expr,
                output: $output:expr,
            }
        )+
    ) => {
        pub(super) const CASES: &[super::compatibility::CompatibilityCase] = &[
            $(
                super::compatibility::CompatibilityCase {
                    upstream_commit: super::compatibility::PINNED_COMMIT,
                    module: $module,
                    order: $order,
                    name: stringify!($name),
                    xfail: compatibility_cases!(@xfail $($xfail)?),
                    input: super::compatibility::FixtureInput {
                        control: $control,
                        options: $options,
                        requests: super::compatibility::BBL_OUTPUT,
                    },
                    output: $output,
                },
            )+
        ];

        $(
            #[test]
            $(#[ignore = $xfail])?
            fn $name() {
                super::compatibility::run(&CASES[$order - 1]);
            }
        )+
    };
    (@xfail) => { None };
    (@xfail $reason:literal) => { Some($reason) };
}

pub(super) use compatibility_cases;
