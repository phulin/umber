//! TeX82's irrecoverable-error family: §93 `fatal_error`, §94 `overflow`, and
//! §95 `confusion`.
//!
//! All three share §93's `succumb`, which sets `history:=fatal_error_stop` and
//! calls §81's `jump_out`. `jump_out` is tex.web's only nontrivial `goto`: it
//! "just cuts across all active procedure levels and goes to `end_of_TEX`",
//! where §1332's `close_files_and_terminate` finishes the job. There is no
//! recovery and no resumption.
//!
//! A library engine cannot leave the process, so the canonical equivalent is a
//! distinguished *terminal state* of the session. `FatalError` is that state's
//! payload; the executor latches it, reports it, and ends the job. Nothing is
//! rolled back, because §81 unwinds Pascal's procedure levels without undoing
//! anything the job already committed.

/// One of the terminal states TeX82 reaches through §81's `jump_out`.
///
/// Three are the irrecoverable errors proper (§93's `fatal_error`, §94's
/// `overflow`, §95's `confusion`); the other two are §82's 100-error stop and
/// §84's `X`, which take the same exit without being diagnoses.
///
/// The payloads are `&'static str` because every tex.web call site passes a
/// string literal: `fatal_error`, `overflow`, and `confusion` are all declared
/// `(s:str_number)` and are only ever handed compile-time text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FatalError {
    /// TeX82 §82's 100th recoverable error, after its terminal notice.
    TooManyErrors,
    /// TeX82 §93 `fatal_error(s)`: prints `Emergency stop` with `s` as its one
    /// help line. Raised when the job cannot continue at all, as in §71's
    /// `*** (job aborted, no legal \end found)`.
    EmergencyStop { help: &'static str },
    /// TeX82 §94 `overflow(s,n)`: prints
    /// `TeX capacity exceeded, sorry [s=n]`. `resource` is §94's `s`, the name
    /// of the exhausted array, and `amount` is `n`, its capacity.
    CapacityExceeded { resource: &'static str, amount: i32 },
    /// TeX82 §95 `confusion(s)`: prints `This can't happen (s)`, where `s`
    /// "tells where" the consistency check was violated.
    Confusion { site: &'static str },
    /// TeX82 §84's `X` option, chosen at §83's dialog:
    /// `interaction:=scroll_mode; jump_out`. Alone among these it prints
    /// nothing and leaves `history` where it was -- it is a requested exit,
    /// not a diagnosis -- but it reaches `end_of_TEX` by the same `jump_out`
    /// and so ends the session identically.
    Quit,
}

impl From<tex_state::print::JumpOut> for FatalError {
    /// The error channel reaches §81's `jump_out` from three places and
    /// reports which through [`tex_state::print::JumpOut`]; each maps onto
    /// the terminal state that already carries it.
    fn from(jump: tex_state::print::JumpOut) -> Self {
        match jump {
            tex_state::print::JumpOut::TooManyErrors => Self::TooManyErrors,
            tex_state::print::JumpOut::EmergencyStop { help } => Self::EmergencyStop { help },
            tex_state::print::JumpOut::Quit => Self::Quit,
        }
    }
}

impl FatalError {
    /// TeX82 §93 `fatal_error(s)`.
    #[must_use]
    pub const fn emergency_stop(help: &'static str) -> Self {
        Self::EmergencyStop { help }
    }

    /// TeX82 §94 `overflow(s,n)`.
    #[must_use]
    pub const fn overflow(resource: &'static str, amount: i32) -> Self {
        Self::CapacityExceeded { resource, amount }
    }

    /// TeX82 §95 `confusion(s)`.
    #[must_use]
    pub const fn confusion(site: &'static str) -> Self {
        Self::Confusion { site }
    }

    /// The canonical diagnostic identity, matching the tex.web procedure name.
    #[must_use]
    pub const fn diagnostic(self) -> &'static str {
        match self {
            Self::TooManyErrors => "too-many-errors",
            Self::EmergencyStop { .. } => "emergency-stop",
            Self::CapacityExceeded { .. } => "capacity-exceeded",
            Self::Confusion { .. } => "confusion",
            Self::Quit => "quit",
        }
    }

    /// The stable observation label, `diagnostic(argument)`.
    ///
    /// This is the identity a detached conformance observer pins; the terminal
    /// wording of §93/§94/§95 stays executor/host formatting policy exactly as
    /// every other [`crate::DiagnosticRecord`] does.
    #[must_use]
    pub fn label(self) -> String {
        match self {
            Self::TooManyErrors => "too-many-errors".into(),
            Self::EmergencyStop { help } => format!("emergency-stop({help})"),
            Self::CapacityExceeded { resource, amount } => {
                format!("capacity-exceeded({resource}={amount})")
            }
            Self::Confusion { site } => format!("confusion({site})"),
            Self::Quit => "quit".into(),
        }
    }

    /// The committed observation for this fatal error.
    #[must_use]
    pub fn record(self) -> crate::DiagnosticRecord {
        crate::DiagnosticRecord {
            severity: FATAL_SEVERITY,
            diagnostic: self.diagnostic(),
            arguments: match self {
                Self::TooManyErrors | Self::Quit => Vec::new(),
                _ => vec![crate::DiagnosticArgument::Name(self.argument())],
            },
        }
    }

    fn argument(self) -> String {
        match self {
            Self::TooManyErrors | Self::Quit => {
                unreachable!("{} has no diagnostic argument", self.diagnostic())
            }
            Self::EmergencyStop { help } => help.into(),
            Self::CapacityExceeded { resource, amount } => format!("{resource}={amount}"),
            Self::Confusion { site } => site.into(),
        }
    }
}

/// The severity every §93 `succumb` diagnostic carries. It is distinct from
/// ordinary recoverable severities precisely because `history` reaches
/// §76's `fatal_error_stop` and the job never resumes.
pub const FATAL_SEVERITY: &str = "fatal";

impl std::fmt::Display for FatalError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.label())
    }
}

#[cfg(test)]
#[path = "fatal/tests.rs"]
mod tests;
