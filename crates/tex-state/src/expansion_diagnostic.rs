//! Detached recoverable diagnostics shared by expansion and execution.

use crate::token::{Token, TracedTokenWord};

/// A TeX error that expansion reports and recovers from without aborting the
/// enclosing scanner. Execution owns presentation of these detached values.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoverableExpansionDiagnostic {
    UndefinedControlSequence {
        name: String,
        context: TracedTokenWord,
    },
    MacroDoesNotMatchDefinition {
        macro_name: String,
        context: TracedTokenWord,
    },
    FileEndedWhileScanningMacro {
        macro_name: String,
        context: TracedTokenWord,
        partial: Vec<Token>,
    },
    InvalidTheTarget {
        context: TracedTokenWord,
    },
    MissingGeneralTextBeginGroup {
        context: TracedTokenWord,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::OriginId;

    #[test]
    fn recoverable_diagnostics_are_detached_cloneable_values() {
        let diagnostic = RecoverableExpansionDiagnostic::MissingGeneralTextBeginGroup {
            context: TracedTokenWord::pack(
                crate::token::Token::Char {
                    ch: 'x',
                    cat: crate::token::Catcode::Letter,
                },
                OriginId::UNKNOWN,
            ),
        };
        assert_eq!(diagnostic.clone(), diagnostic);
        let RecoverableExpansionDiagnostic::MissingGeneralTextBeginGroup { context } = diagnostic
        else {
            unreachable!()
        };
        assert_eq!(context.origin(), OriginId::UNKNOWN);
    }
}
