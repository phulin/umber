use std::fmt::Write as _;

use bib_model::{
    Entry, FieldProvenance, FieldValue, GeneratedFile, OutputFormat, OutputRequest,
    ProcessedSection,
};

use crate::{
    DotOutputFailure, DotOutputFailureKind, OutputContext, OutputPlan, OutputRouter,
    router::failure as output_failure,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DotInclude {
    pub sections: bool,
    pub fields: bool,
    pub xdata: bool,
    pub crossrefs: bool,
    pub xrefs: bool,
    pub related: bool,
}

impl Default for DotInclude {
    fn default() -> Self {
        Self {
            sections: true,
            fields: true,
            xdata: true,
            crossrefs: true,
            xrefs: true,
            related: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DotOptions {
    include: DotInclude,
}

impl DotOptions {
    #[must_use]
    pub const fn with_include(mut self, include: DotInclude) -> Self {
        self.include = include;
        self
    }

    #[must_use]
    pub const fn include(self) -> DotInclude {
        self.include
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DotSerializer {
    options: DotOptions,
}

impl DotSerializer {
    #[must_use]
    pub const fn new(options: DotOptions) -> Self {
        Self { options }
    }

    #[must_use]
    pub const fn options(self) -> DotOptions {
        self.options
    }

    fn write_section(
        self,
        text: &mut String,
        section: &ProcessedSection,
        max_bytes: usize,
    ) -> Result<(), DotOutputFailure> {
        let section_number = section.id().get();
        if self.options.include.sections {
            writeln!(text, "  subgraph \"cluster_section{section_number}\" {{")
                .expect("String writes cannot fail");
            writeln!(text, "    label=\"Section {section_number}\";")
                .expect("String writes cannot fail");
            writeln!(text, "    tooltip=\"Section {section_number}\";")
                .expect("String writes cannot fail");
            text.push_str(
                "    fontsize=\"10\";\n    fontname=serif;\n    fillcolor=\"#fce3fa\";\n\n",
            );
        }
        check_limit(text, max_bytes)?;

        for entry in section.entries() {
            self.write_entry(text, section_number, entry, max_bytes)?;
        }
        if self.options.include.sections {
            text.push_str("  }\n\n");
        }
        self.write_edges(text, section, max_bytes)
    }

    fn write_entry(
        self,
        text: &mut String,
        section: u32,
        entry: &Entry,
        max_bytes: usize,
    ) -> Result<(), DotOutputFailure> {
        let id = escape(entry.id().as_str());
        let entry_type = escape(&entry.entry_type().as_str().to_ascii_uppercase());
        let indent = if self.options.include.sections {
            "    "
        } else {
            "  "
        };
        writeln!(
            text,
            "{indent}subgraph \"cluster_section{section}/{id}\" {{"
        )
        .expect("String writes cannot fail");
        writeln!(text, "{indent}  fontsize=\"10\";").expect("String writes cannot fail");
        writeln!(text, "{indent}  label=\"{id} ({entry_type})\";")
            .expect("String writes cannot fail");
        writeln!(text, "{indent}  tooltip=\"{id} ({entry_type})\";")
            .expect("String writes cannot fail");
        let fill = if entry.entry_type().as_str().eq_ignore_ascii_case("xdata") {
            "#deefff"
        } else {
            "#a0d0ff"
        };
        writeln!(text, "{indent}  fillcolor=\"{fill}\";\n").expect("String writes cannot fail");
        if self.options.include.fields {
            for field in entry.fields().iter() {
                let field_id = escape(field.id().as_str());
                let label = escape(&field.id().as_str().to_ascii_uppercase());
                writeln!(
                    text,
                    "{indent}  \"section{section}/{id}/{field_id}\" [ label=\"{label}\" ]"
                )
                .expect("String writes cannot fail");
                check_limit(text, max_bytes)?;
            }
        }
        writeln!(text, "{indent}}}\n").expect("String writes cannot fail");
        check_limit(text, max_bytes)
    }

    fn write_edges(
        self,
        text: &mut String,
        section: &ProcessedSection,
        max_bytes: usize,
    ) -> Result<(), DotOutputFailure> {
        let section_number = section.id().get();
        for entry in section.entries() {
            for field in entry.fields().iter() {
                match field.provenance() {
                    FieldProvenance::Inherited { parent, .. } if self.options.include.crossrefs => {
                        write_edge(
                            text,
                            section_number,
                            parent.entry().as_str(),
                            parent.field().as_str(),
                            entry.id().as_str(),
                            field.id().as_str(),
                            "#7d7879",
                            &format!(
                                "{}/{} inherited from {}/{}",
                                entry.id(),
                                field.id().as_str().to_ascii_uppercase(),
                                parent.entry(),
                                parent.field().as_str().to_ascii_uppercase()
                            ),
                            false,
                        );
                    }
                    FieldProvenance::Computed { inputs, .. } => {
                        for input in inputs {
                            write_edge(
                                text,
                                section_number,
                                input.entry().as_str(),
                                input.field().as_str(),
                                entry.id().as_str(),
                                field.id().as_str(),
                                "#2ca314",
                                &format!(
                                    "{}/{} derived from {}/{}",
                                    entry.id(),
                                    field.id().as_str().to_ascii_uppercase(),
                                    input.entry(),
                                    input.field().as_str().to_ascii_uppercase()
                                ),
                                false,
                            );
                        }
                    }
                    _ => {}
                }
                self.write_relationship_edges(text, section_number, entry, field);
                check_limit(text, max_bytes)?;
            }
        }
        Ok(())
    }

    fn write_relationship_edges(
        self,
        text: &mut String,
        section: u32,
        entry: &Entry,
        field: &bib_model::Field,
    ) {
        let name = field.id().as_str();
        let enabled = match name {
            "xdata" => self.options.include.xdata,
            "crossref" => self.options.include.crossrefs,
            "xref" => self.options.include.xrefs,
            "related" => self.options.include.related,
            _ => false,
        };
        if !enabled {
            return;
        }
        let Some(targets) = relationship_targets(field.value()) else {
            return;
        };
        let color = if name == "related" {
            "#ad1741"
        } else {
            "#7d7879"
        };
        for target in targets {
            write_edge(
                text,
                section,
                entry.id().as_str(),
                field.id().as_str(),
                target,
                "title",
                color,
                &format!("{} {}S {target}", entry.id(), name.to_ascii_uppercase()),
                true,
            );
        }
    }
}

impl DotSerializer {
    pub fn serialize(
        &self,
        context: OutputContext<'_>,
        request: &OutputRequest,
    ) -> Result<GeneratedFile, DotOutputFailure> {
        OutputRouter::new(crate::OutputOptions::default().with_dot(self.options)).serialize_as(
            OutputFormat::Dot,
            context,
            request,
        )
    }

    fn render(self, plan: &OutputPlan<'_>) -> Result<String, DotOutputFailure> {
        let mut text = String::from(
            "digraph Biberdata {\n  compound = true;\n  edge [ arrowhead=open ];\n  graph [ style=filled, rankdir=LR ];\n  node [\n    fontsize=10,\n    fillcolor=white,\n    style=filled,\n    shape=box ];\n\n",
        );
        check_limit(&text, plan.request().max_bytes())?;
        for section in plan.sections() {
            self.write_section(&mut text, section.section(), plan.request().max_bytes())?;
        }
        text.push_str("}\n");
        check_limit(&text, plan.request().max_bytes())?;
        Ok(text)
    }
}

pub(crate) fn render(plan: &OutputPlan<'_>) -> Result<String, DotOutputFailure> {
    DotSerializer::new(plan.options().dot()).render(plan)
}

fn relationship_targets(value: &FieldValue) -> Option<Vec<&str>> {
    match value {
        FieldValue::Literal(value) => Some(vec![value.as_str()]),
        FieldValue::KeyList(values) => Some(values.iter().map(|value| value.as_str()).collect()),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
fn write_edge(
    text: &mut String,
    section: u32,
    from_entry: &str,
    from_field: &str,
    to_entry: &str,
    to_field: &str,
    color: &str,
    tooltip: &str,
    dashed: bool,
) {
    let style = if dashed { " style=\"dashed\"," } else { "" };
    writeln!(
        text,
        "  \"section{section}/{}/{}\" -> \"section{section}/{}/{}\" [{style} penwidth=\"2.0\", color=\"{color}\", tooltip=\"{}\" ]",
        escape(from_entry),
        escape(from_field),
        escape(to_entry),
        escape(to_field),
        escape(tooltip),
    )
    .expect("String writes cannot fail");
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}

fn check_limit(text: &str, max_bytes: usize) -> Result<(), DotOutputFailure> {
    if text.len() > max_bytes {
        Err(limit_failure(max_bytes))
    } else {
        Ok(())
    }
}

fn limit_failure(max_bytes: usize) -> DotOutputFailure {
    failure(
        DotOutputFailureKind::Limit,
        "BIB_OUTPUT_LIMIT",
        &format!("DOT output exceeds the configured {max_bytes}-byte limit"),
    )
}

fn failure(kind: DotOutputFailureKind, code: &str, message: &str) -> DotOutputFailure {
    output_failure(OutputFormat::Dot, kind, code, message)
}
