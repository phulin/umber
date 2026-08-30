//! TeX group-boundary values independent of command interpretation.

/// TeX group boundary kind tracked by the ordered save journal.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GroupKind {
    Simple,
    HBox,
    AdjustedHBox,
    VBox,
    VTop,
    SemiSimple,
    MathShift,
    Align,
    NoAlign,
    Output,
    Math,
    Disc,
    Insert,
    VCenter,
    MathChoice,
    MathLeft,
}

/// Detached identity of one live TeX save-stack boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupFrame {
    kind: GroupKind,
    entered_line: u32,
    lineage: u64,
    pub(super) journal_start: u32,
    pub(super) level: u32,
    /// State-owned TeX82 save words immediately before this boundary.
    ///
    /// The matching `GroupExit` uses these two scalars to restore the
    /// incremental diagnostic projection without retaining or copying any
    /// saved value outside the ordered journal.
    pub(crate) save_stack_words_before: usize,
    pub(crate) latest_save_push_before: Option<(u32, usize)>,
}

impl GroupFrame {
    pub(super) const fn new(
        kind: GroupKind,
        entered_line: u32,
        lineage: u64,
        journal_start: u32,
        level: u32,
        save_stack_words_before: usize,
        latest_save_push_before: Option<(u32, usize)>,
    ) -> Self {
        Self {
            kind,
            entered_line,
            lineage,
            journal_start,
            level,
            save_stack_words_before,
            latest_save_push_before,
        }
    }

    #[cfg(test)]
    pub(crate) const fn for_journal_test(
        kind: GroupKind,
        lineage: u64,
        level: u32,
        save_stack_words_before: usize,
        latest_save_push_before: Option<(u32, usize)>,
    ) -> Self {
        Self::new(
            kind,
            1,
            lineage,
            0,
            level,
            save_stack_words_before,
            latest_save_push_before,
        )
    }

    #[must_use]
    pub const fn kind(self) -> GroupKind {
        self.kind
    }

    #[must_use]
    pub const fn entered_line(self) -> u32 {
        self.entered_line
    }

    #[must_use]
    pub const fn lineage(self) -> u64 {
        self.lineage
    }

    pub(crate) const fn level(self) -> u32 {
        self.level
    }
}

impl GroupKind {
    #[must_use]
    pub const fn start_text(self) -> &'static str {
        match self {
            Self::Simple => "{",
            Self::SemiSimple => "\\begingroup",
            Self::MathShift => "$",
            Self::Align => "an alignment entry",
            Self::HBox
            | Self::AdjustedHBox
            | Self::VBox
            | Self::VTop
            | Self::NoAlign
            | Self::Output
            | Self::Math
            | Self::Disc
            | Self::Insert
            | Self::VCenter
            | Self::MathChoice
            | Self::MathLeft => "{",
        }
    }

    #[must_use]
    pub const fn end_text(self) -> &'static str {
        match self {
            Self::Simple => "}",
            Self::SemiSimple => "\\endgroup",
            Self::MathShift => "$",
            Self::Align => "\\cr",
            Self::HBox
            | Self::AdjustedHBox
            | Self::VBox
            | Self::VTop
            | Self::NoAlign
            | Self::Output
            | Self::Math
            | Self::Disc
            | Self::Insert
            | Self::VCenter
            | Self::MathChoice
            | Self::MathLeft => "}",
        }
    }

    #[must_use]
    pub const fn group_text(self) -> &'static str {
        match self {
            Self::Simple => "simple group",
            Self::HBox => "hbox group",
            Self::AdjustedHBox => "adjusted hbox group",
            Self::VBox => "vbox group",
            Self::VTop => "vtop group",
            Self::Align => "align group",
            Self::NoAlign => "no align group",
            Self::Output => "output group",
            Self::Math => "math group",
            Self::Disc => "disc group",
            Self::Insert => "insert group",
            Self::VCenter => "vcenter group",
            Self::MathChoice => "math choice group",
            Self::SemiSimple => "semi simple group",
            Self::MathShift => "math shift group",
            Self::MathLeft => "math left group",
        }
    }

    #[must_use]
    pub const fn etex_code(self) -> i32 {
        match self {
            Self::Simple => 1,
            Self::HBox => 2,
            Self::AdjustedHBox => 3,
            Self::VBox => 4,
            Self::VTop => 5,
            Self::Align => 6,
            Self::NoAlign => 7,
            Self::Output => 8,
            Self::Math => 9,
            Self::Disc => 10,
            Self::Insert => 11,
            Self::VCenter => 12,
            Self::MathChoice => 13,
            Self::SemiSimple => 14,
            Self::MathShift => 15,
            Self::MathLeft => 16,
        }
    }
}

/// Group mismatch detected before any state change.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GroupMismatch {
    expected: GroupKind,
    actual: Option<GroupKind>,
}

impl GroupMismatch {
    pub(super) const fn new(expected: GroupKind, actual: GroupKind) -> Self {
        Self {
            expected,
            actual: Some(actual),
        }
    }

    pub(super) const fn no_group(expected: GroupKind) -> Self {
        Self {
            expected,
            actual: None,
        }
    }

    #[must_use]
    pub const fn expected(self) -> GroupKind {
        self.expected
    }

    #[must_use]
    pub const fn actual(self) -> Option<GroupKind> {
        self.actual
    }
}
