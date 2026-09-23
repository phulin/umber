use super::*;

/// TeX82 §§511–520's three-part current filename.
#[derive(Clone, Debug, Default, Eq, Hash, PartialEq)]
pub struct FileNameComponents {
    pub area: String,
    pub name: String,
    pub extension: String,
}

impl FileNameComponents {
    /// Applies TeX82 §§516--519's platform-independent component scan to an
    /// already selected startup name.
    #[must_use]
    pub fn from_tex_name(value: &str) -> Self {
        let mut components = Self::default();
        for ch in value.chars() {
            components.push_character(ch);
        }
        components
    }

    #[must_use]
    pub fn packed(&self) -> String {
        format!("{}{}{}", self.area, self.name, self.extension)
    }

    pub fn apply_default_extension(&mut self, extension: &str) {
        if self.extension.is_empty() {
            self.extension.push_str(extension);
        }
    }

    pub(crate) fn push_character(&mut self, ch: char) {
        match ch {
            '/' | '\\' | ':' => {
                self.area.push_str(&self.name);
                self.area.push_str(&self.extension);
                self.area.push(ch);
                self.name.clear();
                self.extension.clear();
            }
            // TeX82 §§516--519: the first dot after the final area
            // delimiter starts `cur_ext`; later dots stay in that same
            // component.
            '.' => self.extension.push(ch),
            _ if self.extension.is_empty() => self.name.push(ch),
            _ => self.extension.push(ch),
        }
    }
}

/// A filename scanned from expanded command-owned input.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ScannedFileName {
    pub components: FileNameComponents,
    pub provenance: StructuredProvenance,
}

impl ScannedFileName {
    #[must_use]
    pub fn packed(&self) -> String {
        self.components.packed()
    }
}

pub(crate) const FILE_NAME_POOL_CAPACITY: usize = 32_000;

/// Completed input-stream operation.  The command core owns every operand;
/// replay only acquires an already-registered immutable resource and mutates
/// World stream state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InputStreamRequest {
    Open {
        stream: i32,
        /// The unrecovered §435 `scan_int` result for `int_error`.
        scanned: i32,
        /// Whether §435 replaced `scanned` with stream zero.
        recovered: bool,
        file_name: ScannedFileName,
    },
    Close {
        stream: i32,
        scanned: i32,
        recovered: bool,
    },
    /// TeX82 §482's `read_toks` has already run: the collected list is
    /// carried here, not a stream the executor must go read itself.
    ///
    /// §1225 calls `read_toks(n,r)` inside `prefixed_command`, so the
    /// collection is part of scanning `\\read` and belongs to the command
    /// core. Replay only installs the parameterless macro §482 built.
    Read {
        /// §1225's plain `scan_int`, unrestricted: §482 maps anything outside
        /// `0..=15` onto stream 16 (the terminal) without diagnosing it.
        stream: i32,
        target: Symbol,
        /// Effective TeX82 §1214 scope selected by `prefixed_command`
        /// before §1225 enters `read_toks`.
        global: bool,
        /// The parameterless macro definition is the sole mutable attempt
        /// root. Its replacement slice supplies read observation after
        /// publication; no duplicate durable token list is created.
        definition: AttemptDefinitionId,
    },
}

/// A completed TeX82 §53 `\immediate` extension request.
///
/// Command control owns the recursive expanded lookahead and all operand
/// scanning.  In particular, a non-I/O lookahead has already been backed up
/// when `Continue` is returned, so replay never needs raw input access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ImmediateExtension {
    Continue,
    /// The recursive expanded-command lookahead found a PDF-only extension,
    /// whose own pdftex.web case runs `check_pdfoutput` before every operand
    /// scan. The stomach turns this typed command identity into the canonical
    /// DVI-mode error without giving the scanner the diagnostic channel.
    PdfExtensionInDviMode(UnexpandablePrimitive),
    OpenOut {
        /// TeX82 §435's effective stream after `scan_four_bit_int`.
        stream: u8,
        file_name: ScannedFileName,
    },
    Write {
        stream: WriteStreamSelector,
        tokens: AttemptTokenListId,
    },
    CloseOut {
        stream: WriteStreamSelector,
    },
    PdfObject(PdfObjectRequest),
    PdfForm(PdfFormRequest),
    PdfImage(PdfImageRequest),
}

/// TeX82 §§1342/1350's normalized selector stored in a write whatsit.
///
/// Slots 16 and 17 are deliberately represented rather than clamped: they
/// stand for every stream above 15 and every negative stream, respectively.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteStreamSelector {
    Stream(u8),
    AboveRange,
    Negative,
}

impl WriteStreamSelector {
    #[must_use]
    pub const fn normalized_number(self) -> i32 {
        match self {
            Self::Stream(slot) => slot as i32,
            Self::AboveRange => 16,
            Self::Negative => 17,
        }
    }

    pub fn stream_slot(self) -> Option<tex_state::world::StreamSlot> {
        match self {
            Self::Stream(slot) => Some(tex_state::world::StreamSlot::new(slot)),
            Self::AboveRange | Self::Negative => None,
        }
    }
}

/// One successfully opened capability-registered input source.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RegisteredInput {
    pub file_name: ScannedFileName,
    pub source: SourceId,
    pub bytes: tex_state::SharedBytes,
}

/// The typed result of TeX82's `init_col` entry lookahead.
///
/// `\omit` is consumed as that lookahead rather than backed up for the
/// selected u-template. The executor receives this semantic distinction,
/// never the command spelling that established it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignmentCellOpening {
    /// The selected column uses its ordinary u-template.
    Template,
    /// The selected column starts with TeX82's template-free `\omit` path.
    Omit,
}

/// The result of projecting one delivered spelling for TeX82's
/// `get_r_token` (§1215). The spelling is authoritative: cached effective
/// command metadata is only an optional agreement check.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DefinitionTargetProjection {
    Target(tex_state::interner::Symbol),
    Malformed,
}
