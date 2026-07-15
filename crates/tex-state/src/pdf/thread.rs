use super::PdfDestinationIdentity;

/// One article thread in the document object ledger.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PdfThreadRecord {
    identity: PdfDestinationIdentity,
    object: u32,
    beads: Vec<PdfThreadBeadRecord>,
}

impl PdfThreadRecord {
    pub(super) fn new(identity: PdfDestinationIdentity, object: u32) -> Self {
        Self {
            identity,
            object,
            beads: Vec::new(),
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
    pub fn beads(&self) -> &[PdfThreadBeadRecord] {
        &self.beads
    }

    pub(super) fn push_bead(&mut self, bead: PdfThreadBeadRecord) {
        self.beads.push(bead);
    }
}

/// Indirect identities allocated for one article bead and its rectangle.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfThreadBeadRecord {
    bead_object: u32,
    rectangle_object: u32,
}

impl PdfThreadBeadRecord {
    pub(super) const fn new(bead_object: u32, rectangle_object: u32) -> Self {
        Self {
            bead_object,
            rectangle_object,
        }
    }
    #[must_use]
    pub const fn bead_object(self) -> u32 {
        self.bead_object
    }
    #[must_use]
    pub const fn rectangle_object(self) -> u32 {
        self.rectangle_object
    }
}
