use umber_vfs::VirtualPath;

use crate::{EntryId, FieldId, TransformationId};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SourceSpan {
    pub byte_start: u64,
    pub byte_end: u64,
    pub line: u32,
    pub column: u32,
}

impl SourceSpan {
    #[must_use]
    pub const fn is_valid(self) -> bool {
        self.byte_start <= self.byte_end && self.line > 0 && self.column > 0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibSourceLocation {
    path: VirtualPath,
    span: SourceSpan,
}

impl BibSourceLocation {
    pub fn new(path: VirtualPath, span: SourceSpan) -> Result<Self, &'static str> {
        if !span.is_valid() {
            return Err("source span must be ordered and use one-based line and column values");
        }
        Ok(Self { path, span })
    }

    #[must_use]
    pub const fn path(&self) -> &VirtualPath {
        &self.path
    }

    #[must_use]
    pub const fn span(&self) -> SourceSpan {
        self.span
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedFrom {
    entry: EntryId,
    field: FieldId,
}

impl DerivedFrom {
    #[must_use]
    pub const fn new(entry: EntryId, field: FieldId) -> Self {
        Self { entry, field }
    }

    #[must_use]
    pub const fn entry(&self) -> &EntryId {
        &self.entry
    }

    #[must_use]
    pub const fn field(&self) -> &FieldId {
        &self.field
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FieldProvenance {
    Datasource(BibSourceLocation),
    Transformed {
        source: BibSourceLocation,
        transformation: TransformationId,
    },
    Inherited {
        source: BibSourceLocation,
        parent: DerivedFrom,
    },
    Computed {
        transformation: TransformationId,
        inputs: Vec<DerivedFrom>,
    },
}
