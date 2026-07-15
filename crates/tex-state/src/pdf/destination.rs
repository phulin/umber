//! Checkpointed regular and structure destination identities.

/// A pdfTeX destination identity. Numeric and byte-name domains never alias.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PdfDestinationIdentity {
    Name(Vec<u8>),
    Number(u32),
}

/// One canonical destination object reservation.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct PdfDestinationRecord {
    identity: PdfDestinationIdentity,
    object: u32,
    structure: Option<u32>,
    defined: bool,
}

impl PdfDestinationRecord {
    pub(super) fn reserved(identity: PdfDestinationIdentity, object: u32) -> Self {
        Self {
            identity,
            object,
            structure: None,
            defined: false,
        }
    }

    #[must_use]
    pub fn identity(&self) -> &PdfDestinationIdentity {
        &self.identity
    }

    #[must_use]
    pub const fn object(&self) -> u32 {
        self.object
    }

    #[must_use]
    pub const fn structure(&self) -> Option<u32> {
        self.structure
    }

    #[must_use]
    pub const fn defined(&self) -> bool {
        self.defined
    }

    pub(super) fn define(&mut self, structure: Option<u32>) -> bool {
        if self.defined {
            return false;
        }
        self.defined = true;
        self.structure = structure;
        true
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfDestinationDefinition {
    pub record: PdfDestinationRecord,
    pub duplicate: bool,
}
