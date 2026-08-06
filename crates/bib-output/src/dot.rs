use bib_model::{
    Entry, FieldProvenance, FieldValue, GeneratedFile, OutputFormat, OutputRequest,
    ProcessedSection,
};

use crate::{DotOutputFailure, OutputContext, OutputPlan, OutputRouter, router::OutputSink};

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
        text: &mut OutputSink<'_>,
        section: &ProcessedSection,
    ) -> Result<(), DotOutputFailure> {
        let section_number = section.id().get();
        if self.options.include.sections {
            text.line(&format!(
                "  subgraph \"cluster_section{section_number}\" {{"
            ))?;
            text.line(&format!("    label=\"Section {section_number}\";"))?;
            text.line(&format!("    tooltip=\"Section {section_number}\";"))?;
            text.push("    fontsize=\"10\";\n    fontname=serif;\n    fillcolor=\"#fce3fa\";\n\n")?;
        }

        for entry in section.entries() {
            self.write_entry(text, section_number, entry)?;
        }
        if self.options.include.sections {
            text.push("  }\n\n")?;
        }
        self.write_edges(text, section)
    }

    fn write_entry(
        self,
        text: &mut OutputSink<'_>,
        section: u32,
        entry: &Entry,
    ) -> Result<(), DotOutputFailure> {
        let id = escape(entry.id().as_str());
        let entry_type = escape(&entry.entry_type().as_str().to_ascii_uppercase());
        let indent = if self.options.include.sections {
            "    "
        } else {
            "  "
        };
        text.line(&format!(
            "{indent}subgraph \"cluster_section{section}/{id}\" {{"
        ))?;
        text.line(&format!("{indent}  fontsize=\"10\";"))?;
        text.line(&format!("{indent}  label=\"{id} ({entry_type})\";"))?;
        text.line(&format!("{indent}  tooltip=\"{id} ({entry_type})\";"))?;
        let fill = if entry.entry_type().as_str().eq_ignore_ascii_case("xdata") {
            "#deefff"
        } else {
            "#a0d0ff"
        };
        text.line(&format!("{indent}  fillcolor=\"{fill}\";\n"))?;
        if self.options.include.fields {
            for field in entry.fields().iter() {
                let field_id = escape(field.id().as_str());
                let label = escape(&field.id().as_str().to_ascii_uppercase());
                text.line(&format!(
                    "{indent}  \"section{section}/{id}/{field_id}\" [ label=\"{label}\" ]"
                ))?;
            }
        }
        text.line(&format!("{indent}}}\n"))
    }

    fn write_edges(
        self,
        text: &mut OutputSink<'_>,
        section: &ProcessedSection,
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
                        )?;
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
                            )?;
                        }
                    }
                    _ => {}
                }
                self.write_relationship_edges(text, section_number, entry, field)?;
            }
        }
        Ok(())
    }

    fn write_relationship_edges(
        self,
        text: &mut OutputSink<'_>,
        section: u32,
        entry: &Entry,
        field: &bib_model::Field,
    ) -> Result<(), DotOutputFailure> {
        let name = field.id().as_str();
        let enabled = match name {
            "xdata" => self.options.include.xdata,
            "crossref" => self.options.include.crossrefs,
            "xref" => self.options.include.xrefs,
            "related" => self.options.include.related,
            _ => false,
        };
        if !enabled {
            return Ok(());
        }
        let Some(targets) = relationship_targets(field.value()) else {
            return Ok(());
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
            )?;
        }
        Ok(())
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

    fn render(
        self,
        plan: &OutputPlan<'_>,
        text: &mut OutputSink<'_>,
    ) -> Result<(), DotOutputFailure> {
        text.push(
            "digraph Biberdata {\n  compound = true;\n  edge [ arrowhead=open ];\n  graph [ style=filled, rankdir=LR ];\n  node [\n    fontsize=10,\n    fillcolor=white,\n    style=filled,\n    shape=box ];\n\n",
        )?;
        for section in plan.sections() {
            self.write_section(text, section)?;
        }
        text.push("}\n")
    }
}

pub(crate) fn render(
    plan: &OutputPlan<'_>,
    sink: &mut OutputSink<'_>,
) -> Result<(), DotOutputFailure> {
    DotSerializer::new(plan.options().dot()).render(plan, sink)
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
    text: &mut OutputSink<'_>,
    section: u32,
    from_entry: &str,
    from_field: &str,
    to_entry: &str,
    to_field: &str,
    color: &str,
    tooltip: &str,
    dashed: bool,
) -> Result<(), DotOutputFailure> {
    let style = if dashed { " style=\"dashed\"," } else { "" };
    text.line(&format!(
        "  \"section{section}/{}/{}\" -> \"section{section}/{}/{}\" [{style} penwidth=\"2.0\", color=\"{color}\", tooltip=\"{}\" ]",
        escape(from_entry),
        escape(from_field),
        escape(to_entry),
        escape(to_field),
        escape(tooltip),
    ))
}

fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ")
}
