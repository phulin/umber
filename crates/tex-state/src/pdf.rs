//! Checkpointed pdfTeX document allocation ledger.

use crate::ContentHash;
use crate::state_hash::{StateHashFragment, StateHasher};

const PDF_STATE_DOMAIN: u64 = 0x7064_665f_7374_6174;
const PDF_PAGE_DOMAIN: u64 = 0x7064_665f_7061_6765;
pub const PDF_CATALOG_OBJECT_ID: u32 = 1;
pub const PDF_PAGES_OBJECT_ID: u32 = 2;
const FIRST_DYNAMIC_OBJECT: u32 = 3;
const OBJECTS_PER_PAGE: u32 = 3;
const MAX_OBJECT_ID: u32 = i32::MAX as u32;

/// pdfTeX output controls frozen by the first shipped page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfOutputParameters {
    pub output: i32,
    pub major_version: i32,
    pub minor_version: i32,
    pub compress_level: i32,
    pub object_compress_level: i32,
    pub decimal_digits: i32,
}

impl PdfOutputParameters {
    /// Applies pdfTeX's first-PDF-write recovery and clamping policy.
    #[must_use]
    pub fn normalized(self) -> Self {
        let major_version = self.major_version.max(1);
        let minor_version = if (0..=9).contains(&self.minor_version) {
            self.minor_version
        } else {
            4
        };
        let mut object_compress_level = self.object_compress_level.clamp(0, 3);
        if major_version == 1 && minor_version < 5 {
            object_compress_level = 0;
        }
        Self {
            major_version,
            minor_version,
            object_compress_level,
            decimal_digits: self.decimal_digits.clamp(0, 4),
            ..self
        }
    }
}

/// Stable object identities assigned to one committed PDF page.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct PdfPageRecord {
    artifact: ContentHash,
    resources_object: u32,
    contents_object: u32,
    page_object: u32,
}

impl PdfPageRecord {
    #[must_use]
    pub const fn artifact(self) -> ContentHash {
        self.artifact
    }
    #[must_use]
    pub const fn resources_object(self) -> u32 {
        self.resources_object
    }
    #[must_use]
    pub const fn contents_object(self) -> u32 {
        self.contents_object
    }
    #[must_use]
    pub const fn page_object(self) -> u32 {
        self.page_object
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct PdfStateCursor {
    enabled: bool,
    next_object: u32,
    page_count: usize,
    output_parameters: Option<PdfOutputParameters>,
    fingerprint: u64,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PdfStateSnapshot(PdfStateCursor);

/// Live append-only PDF allocation state owned by one Universe timeline.
#[derive(Clone, Debug)]
pub(crate) struct PdfState {
    enabled: bool,
    next_object: u32,
    pages: Vec<PdfPageRecord>,
    output_parameters: Option<PdfOutputParameters>,
    fingerprint: u64,
}

impl Default for PdfState {
    fn default() -> Self {
        Self {
            enabled: false,
            next_object: FIRST_DYNAMIC_OBJECT,
            pages: Vec::new(),
            output_parameters: None,
            fingerprint: base_fingerprint(false),
        }
    }
}

impl PdfState {
    pub(crate) fn enable(&mut self) {
        if self.enabled {
            return;
        }
        debug_assert!(self.pages.is_empty());
        self.enabled = true;
        self.next_object = FIRST_DYNAMIC_OBJECT;
        self.fingerprint = base_fingerprint(true);
    }

    #[must_use]
    pub(crate) const fn enabled(&self) -> bool {
        self.enabled
    }
    #[must_use]
    pub(crate) fn pages(&self) -> &[PdfPageRecord] {
        &self.pages
    }
    #[must_use]
    pub(crate) const fn next_object(&self) -> u32 {
        self.next_object
    }
    #[must_use]
    pub(crate) const fn is_format_empty(&self) -> bool {
        self.pages.is_empty()
            && self.next_object == FIRST_DYNAMIC_OBJECT
            && self.output_parameters.is_none()
    }

    pub(crate) fn ensure_page_capacity(&self, parameters: PdfOutputParameters) -> Result<(), ()> {
        if !self.enabled || self.output_parameters.unwrap_or(parameters).output <= 0 {
            return Ok(());
        }
        let last = self
            .next_object
            .checked_add(OBJECTS_PER_PAGE - 1)
            .ok_or(())?;
        (last <= MAX_OBJECT_ID).then_some(()).ok_or(())
    }

    pub(crate) fn commit_page(&mut self, artifact: ContentHash, parameters: PdfOutputParameters) {
        if !self.enabled {
            return;
        }
        let parameters = match self.output_parameters {
            Some(parameters) => parameters,
            None => {
                self.output_parameters = Some(parameters);
                self.fingerprint = freeze_fingerprint(self.fingerprint, parameters);
                parameters
            }
        };
        if parameters.output <= 0 {
            return;
        }
        self.ensure_page_capacity(parameters)
            .expect("PDF page object capacity was preflighted");
        let record = PdfPageRecord {
            artifact,
            resources_object: self.next_object,
            contents_object: self.next_object + 1,
            page_object: self.next_object + 2,
        };
        self.next_object += OBJECTS_PER_PAGE;
        self.pages.push(record);
        self.fingerprint = append_fingerprint(self.fingerprint, record);
    }

    #[must_use]
    pub(crate) const fn output_parameters(&self) -> Option<PdfOutputParameters> {
        self.output_parameters
    }

    #[must_use]
    pub(crate) const fn cursor(&self) -> PdfStateCursor {
        PdfStateCursor {
            enabled: self.enabled,
            next_object: self.next_object,
            page_count: self.pages.len(),
            output_parameters: self.output_parameters,
            fingerprint: self.fingerprint,
        }
    }
    #[must_use]
    pub(crate) const fn snapshot(&self) -> PdfStateSnapshot {
        PdfStateSnapshot(self.cursor())
    }

    pub(crate) fn rollback(&mut self, snapshot: PdfStateSnapshot) {
        let cursor = snapshot.0;
        assert!(
            cursor.page_count <= self.pages.len(),
            "PDF snapshot suffix was discarded"
        );
        self.pages.truncate(cursor.page_count);
        self.enabled = cursor.enabled;
        self.next_object = cursor.next_object;
        self.output_parameters = cursor.output_parameters;
        self.fingerprint = cursor.fingerprint;
    }

    #[must_use]
    pub(crate) fn hash_fragment(&self) -> StateHashFragment {
        let cursor = self.cursor();
        StateHashFragment::from_builder(PDF_STATE_DOMAIN, |hasher| {
            hasher.bool(cursor.enabled);
            hasher.u32(cursor.next_object);
            hasher.usize(cursor.page_count);
            hash_output_parameters(hasher, cursor.output_parameters);
            hasher.u64(cursor.fingerprint);
        })
    }
}

fn base_fingerprint(enabled: bool) -> u64 {
    let mut hasher = StateHasher::new(PDF_STATE_DOMAIN);
    hasher.bool(enabled);
    hasher.u32(FIRST_DYNAMIC_OBJECT);
    hasher.finish()
}

fn freeze_fingerprint(previous: u64, parameters: PdfOutputParameters) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hash_output_parameters(&mut hasher, Some(parameters));
    hasher.finish()
}

fn append_fingerprint(previous: u64, record: PdfPageRecord) -> u64 {
    let mut hasher = StateHasher::new(PDF_PAGE_DOMAIN);
    hasher.u64(previous);
    hasher.bytes(&record.artifact.bytes());
    hasher.u32(record.resources_object);
    hasher.u32(record.contents_object);
    hasher.u32(record.page_object);
    hasher.finish()
}

fn hash_output_parameters(hasher: &mut StateHasher, parameters: Option<PdfOutputParameters>) {
    hasher.bool(parameters.is_some());
    if let Some(parameters) = parameters {
        hasher.i32(parameters.output);
        hasher.i32(parameters.major_version);
        hasher.i32(parameters.minor_version);
        hasher.i32(parameters.compress_level);
        hasher.i32(parameters.object_compress_level);
        hasher.i32(parameters.decimal_digits);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rollback_reuses_page_object_suffix_and_fingerprint() {
        let mut state = PdfState::default();
        state.enable();
        let snapshot = state.snapshot();
        let hash = ContentHash::new([7; 32]);
        let parameters = PdfOutputParameters {
            output: 1,
            major_version: 1,
            minor_version: 4,
            compress_level: 9,
            object_compress_level: 0,
            decimal_digits: 3,
        };
        state.commit_page(hash, parameters);
        let first = (state.pages()[0], state.cursor());
        state.rollback(snapshot);
        state.commit_page(hash, parameters);
        assert_eq!((state.pages()[0], state.cursor()), first);
    }
}
