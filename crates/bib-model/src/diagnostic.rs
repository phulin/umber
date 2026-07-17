use std::fmt;

use crate::{BibSourceLocation, EntryId, FieldId};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum BibSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct BibDiagnosticCode(String);

impl BibDiagnosticCode {
    pub fn new(value: impl Into<String>) -> Result<Self, DiagnosticError> {
        let value = value.into();
        if value.is_empty()
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(DiagnosticError(
                "diagnostic codes use nonempty ASCII A-Z, 0-9, and underscore",
            ));
        }
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiagnosticError(pub &'static str);

impl fmt::Display for DiagnosticError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for DiagnosticError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BibDiagnostic {
    code: BibDiagnosticCode,
    severity: BibSeverity,
    message: String,
    source: Option<BibSourceLocation>,
    entry: Option<EntryId>,
    field: Option<FieldId>,
}

impl BibDiagnostic {
    #[must_use]
    pub const fn code(&self) -> &BibDiagnosticCode {
        &self.code
    }

    #[must_use]
    pub const fn severity(&self) -> BibSeverity {
        self.severity
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    #[must_use]
    pub const fn source(&self) -> Option<&BibSourceLocation> {
        self.source.as_ref()
    }

    #[must_use]
    pub const fn entry(&self) -> Option<&EntryId> {
        self.entry.as_ref()
    }

    #[must_use]
    pub const fn field(&self) -> Option<&FieldId> {
        self.field.as_ref()
    }
}

#[derive(Clone, Debug)]
pub struct DiagnosticBuilder {
    diagnostic: BibDiagnostic,
}

impl DiagnosticBuilder {
    pub fn new(
        code: BibDiagnosticCode,
        severity: BibSeverity,
        message: impl Into<String>,
    ) -> Result<Self, DiagnosticError> {
        let message = message.into();
        if message.is_empty() || message.contains('\0') {
            return Err(DiagnosticError(
                "diagnostic messages must be nonempty and contain no NUL",
            ));
        }
        Ok(Self {
            diagnostic: BibDiagnostic {
                code,
                severity,
                message,
                source: None,
                entry: None,
                field: None,
            },
        })
    }

    pub fn source(&mut self, source: BibSourceLocation) -> &mut Self {
        self.diagnostic.source = Some(source);
        self
    }

    pub fn entry(&mut self, entry: EntryId) -> &mut Self {
        self.diagnostic.entry = Some(entry);
        self
    }

    pub fn field(&mut self, field: FieldId) -> &mut Self {
        self.diagnostic.field = Some(field);
        self
    }

    #[must_use]
    pub fn freeze(self) -> BibDiagnostic {
        self.diagnostic
    }
}
