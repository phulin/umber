use std::collections::BTreeSet;
use std::fmt;
use std::sync::Arc;

use bib_model::{
    BibConfigurationBuilder, DataList, DataListId, Entry, EntryId, GeneratedFile, OutputRequest,
    ProcessedBibliography, ProcessedBibliographyBuilder, ProcessedSectionBuilder, SectionId,
};
use bib_output::{OutputContext, OutputOptions, OutputRouter};
use bib_unicode::UnicodeData;

use crate::COMPATIBILITY_VERSION;

const TOOL_SECTION: u32 = 99_999;
const TOOL_LIST: &str = "tool/global//global/global/global";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolFailureKind {
    DuplicateEntry,
    InvalidOrder,
    DuplicateOutputPath,
    Output,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolFailure {
    kind: ToolFailureKind,
    message: Arc<str>,
}

impl ToolFailure {
    #[must_use]
    pub const fn kind(&self) -> ToolFailureKind {
        self.kind
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ToolFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for ToolFailure {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolResult {
    document: Arc<ProcessedBibliography>,
    files: Arc<[GeneratedFile]>,
}

impl ToolResult {
    #[must_use]
    pub const fn document(&self) -> &Arc<ProcessedBibliography> {
        &self.document
    }

    pub fn files(&self) -> impl ExactSizeIterator<Item = &GeneratedFile> {
        self.files.iter()
    }
}

/// Builds the synthetic section used by bibliography tool mode and writes all
/// requested outputs in process from the resulting frozen document.
#[derive(Clone, Debug, Default)]
pub struct SyntheticTool {
    entries: Vec<Entry>,
    order: Option<Vec<EntryId>>,
    output_options: OutputOptions,
}

impl SyntheticTool {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn entry(&mut self, entry: Entry) -> Result<&mut Self, ToolFailure> {
        if self
            .entries
            .iter()
            .any(|existing| existing.id() == entry.id())
        {
            return Err(failure(
                ToolFailureKind::DuplicateEntry,
                format!("duplicate tool entry `{}`", entry.id()),
            ));
        }
        self.entries.push(entry);
        Ok(self)
    }

    pub fn entries(
        &mut self,
        entries: impl IntoIterator<Item = Entry>,
    ) -> Result<&mut Self, ToolFailure> {
        for entry in entries {
            self.entry(entry)?;
        }
        Ok(self)
    }

    pub fn order(&mut self, entries: impl IntoIterator<Item = EntryId>) -> &mut Self {
        self.order = Some(entries.into_iter().collect());
        self
    }

    pub fn output_options(&mut self, options: OutputOptions) -> &mut Self {
        self.output_options = options;
        self
    }

    pub fn run(
        self,
        requests: impl IntoIterator<Item = OutputRequest>,
    ) -> Result<ToolResult, ToolFailure> {
        let mut section = ProcessedSectionBuilder::new(SectionId::new(TOOL_SECTION));
        let declared = self
            .entries
            .iter()
            .map(|entry| entry.id().clone())
            .collect::<Vec<_>>();
        let order = self.order.unwrap_or_else(|| declared.clone());
        validate_order(&declared, &order)?;
        for entry in self.entries {
            section
                .entry(entry)
                .map_err(|error| failure(ToolFailureKind::DuplicateEntry, error.to_string()))?;
        }
        section
            .list(
                DataList::new(
                    DataListId::new(TOOL_LIST).expect("static tool list identifier"),
                    order,
                )
                .map_err(|error| failure(ToolFailureKind::InvalidOrder, error.to_string()))?,
            )
            .map_err(|error| failure(ToolFailureKind::InvalidOrder, error.to_string()))?;

        let mut document = ProcessedBibliographyBuilder::new(
            BibConfigurationBuilder::new(COMPATIBILITY_VERSION).freeze(),
        );
        document
            .section(section.freeze())
            .expect("a fresh document accepts the synthetic tool section");
        let document = Arc::new(document.freeze());
        let unicode = UnicodeData::pinned();
        let router = OutputRouter::new(self.output_options);
        let mut paths = BTreeSet::new();
        let mut files = Vec::new();
        for request in requests {
            if !paths.insert(request.path().clone()) {
                return Err(failure(
                    ToolFailureKind::DuplicateOutputPath,
                    format!("duplicate tool output path `{}`", request.path()),
                ));
            }
            files.push(
                router
                    .serialize(OutputContext::new(&document, &unicode), &request)
                    .map_err(|error| failure(ToolFailureKind::Output, error.to_string()))?,
            );
        }
        Ok(ToolResult {
            document,
            files: files.into(),
        })
    }
}

fn validate_order(declared: &[EntryId], order: &[EntryId]) -> Result<(), ToolFailure> {
    let declared = declared.iter().collect::<BTreeSet<_>>();
    let ordered = order.iter().collect::<BTreeSet<_>>();
    if declared.len() != order.len() || declared != ordered {
        return Err(failure(
            ToolFailureKind::InvalidOrder,
            "tool order must contain every entry exactly once",
        ));
    }
    Ok(())
}

fn failure(kind: ToolFailureKind, message: impl Into<Arc<str>>) -> ToolFailure {
    ToolFailure {
        kind,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests;
