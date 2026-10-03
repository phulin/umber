use super::*;
use smallvec::SmallVec;
use tex_state::ids::FontId;

#[derive(Clone, Copy)]
pub(crate) struct LigatureSourceEntry {
    pub(crate) ch: char,
    pub(crate) origin: OriginId,
}

#[derive(Clone, Copy)]
pub(crate) struct ProvenanceSpan {
    pub(crate) start: usize,
    pub(crate) len: usize,
}

impl ProvenanceSpan {
    pub(super) const EMPTY: Self = Self { start: 0, len: 0 };

    pub(crate) fn end(self) -> usize {
        self.start + self.len
    }
}

#[derive(Clone, Copy)]
pub(super) struct LigatureBoundaryCell {
    pub(super) code: Option<u8>,
    pub(super) lig_kern_start: Option<u16>,
    pub(super) leading_auto_kern: Scaled,
}

pub(crate) struct LigatureGlyphCell {
    pub(crate) font: FontId,
    pub(crate) ch: char,
    pub(super) code: Option<u8>,
    pub(super) lig_kern_start: Option<u16>,
    pub(super) leading_auto_kern: Scaled,
    pub(super) trailing_auto_kern: Scaled,
    pub(crate) provenance: ProvenanceSpan,
    pub(crate) ligature_present: bool,
    pub(crate) left_hit: bool,
    pub(crate) right_hit: bool,
    pub(super) discard_if_missing: bool,
}

impl LigatureGlyphCell {
    pub(super) fn new<G>(
        stores: &CommandContext<'_, G>,
        font: FontId,
        ch: char,
        provenance: ProvenanceSpan,
        ligature_present: bool,
        left_hit: bool,
        right_hit: bool,
    ) -> Self {
        let code = font_code(ch).ok();
        let lig_kern_start =
            code.and_then(|code| stores.font_lig_kern_start(font, LigKernChar::Char(code)));
        let (leading_auto_kern, trailing_auto_kern) = auto_kern_values(stores, font, code);
        Self {
            font,
            ch,
            code,
            lig_kern_start,
            leading_auto_kern,
            trailing_auto_kern,
            provenance,
            ligature_present,
            left_hit,
            right_hit,
            discard_if_missing: false,
        }
    }

    pub(crate) fn into_pending(self, source: &[LigatureSourceEntry]) -> PendingHRunChar {
        let mut orig = SmallVec::new();
        let mut origins = SmallVec::new();
        for entry in source {
            orig.push(entry.ch);
            origins.push(entry.origin);
        }
        PendingHRunChar {
            font: self.font,
            ch: self.ch,
            orig,
            origins,
            ligature_present: self.ligature_present,
            left_hit: self.left_hit,
            right_hit: self.right_hit,
        }
    }
}

pub(super) enum TfmCell {
    Boundary(LigatureBoundaryCell),
    Glyph(LigatureGlyphCell),
    Kern { amount: Scaled, kind: KernKind },
}

/// Only retaining ligature instructions use the stack. The completed output
/// tape exists solely for hyphenation's physical-suffix queries.
#[derive(Default)]
pub(crate) struct TfmScratch {
    pub(super) stack: Vec<TfmCell>,
    pub(super) output: Vec<Option<TfmCell>>,
    provenance: Vec<LigatureSourceEntry>,
}

impl TfmScratch {
    pub(crate) fn clear(&mut self) {
        self.stack.clear();
        self.output.clear();
        self.provenance.clear();
    }

    pub(super) fn append_source(&mut self, ch: char, origin: OriginId) -> ProvenanceSpan {
        let start = self.provenance.len();
        self.provenance.push(LigatureSourceEntry { ch, origin });
        ProvenanceSpan { start, len: 1 }
    }

    pub(crate) fn source(&self, span: ProvenanceSpan) -> &[LigatureSourceEntry] {
        &self.provenance[span.start..span.end()]
    }

    pub(crate) fn next_is_font_kern(&self, current: usize) -> bool {
        matches!(
            self.output.get(current + 1),
            Some(Some(TfmCell::Kern {
                kind: KernKind::Font,
                ..
            }))
        )
    }

    pub(crate) fn physical_boundary_present(
        &self,
        current: usize,
        current_start: usize,
        current_len: usize,
        boundary: usize,
    ) -> bool {
        let mut position = current_start.saturating_add(current_len);
        if position == boundary {
            return true;
        }
        for cell in self.output.iter().skip(current + 1).flatten() {
            if let TfmCell::Glyph(glyph) = cell {
                position = position.saturating_add(glyph.provenance.len);
            }
            if position == boundary {
                return true;
            }
        }
        false
    }

    pub(crate) fn physical_nodes_through_boundary(
        &self,
        current: usize,
        current_start: usize,
        current_len: usize,
        boundary: usize,
    ) -> usize {
        let mut position = current_start.saturating_add(current_len);
        let mut count = 1;
        if position > boundary {
            return count;
        }
        for cell in self.output.iter().skip(current + 1).flatten() {
            if let TfmCell::Glyph(glyph) = cell {
                position = position.saturating_add(glyph.provenance.len);
            }
            if position > boundary {
                break;
            }
            count += 1;
        }
        count
    }

    pub(super) fn merge_source_spans(
        &mut self,
        left: ProvenanceSpan,
        right: ProvenanceSpan,
    ) -> ProvenanceSpan {
        if left.len == 0 {
            return right;
        }
        if right.len == 0 {
            return left;
        }
        if left.end() == right.start {
            return ProvenanceSpan {
                start: left.start,
                len: left.len + right.len,
            };
        }
        // A merged span can precede newly admitted source entries. Copy only
        // a genuinely noncontiguous concatenation into the provenance tape.
        let start = self.provenance.len();
        self.provenance.extend_from_within(left.start..left.end());
        self.provenance.extend_from_within(right.start..right.end());
        ProvenanceSpan {
            start,
            len: left.len + right.len,
        }
    }
}

pub(super) struct TfmRunLoan<'a> {
    pub(super) scratch: &'a mut TfmScratch,
    restore: (usize, usize, usize),
}

impl<'a> TfmRunLoan<'a> {
    pub(super) fn new(scratch: &'a mut TfmScratch) -> Self {
        scratch.clear();
        Self::nested(scratch)
    }

    pub(super) fn nested(scratch: &'a mut TfmScratch) -> Self {
        let restore = (
            scratch.stack.len(),
            scratch.output.len(),
            scratch.provenance.len(),
        );
        Self { scratch, restore }
    }
}

impl Drop for TfmRunLoan<'_> {
    fn drop(&mut self) {
        let (stack, output, provenance) = self.restore;
        self.scratch.stack.truncate(stack);
        self.scratch.output.truncate(output);
        self.scratch.provenance.truncate(provenance);
    }
}
