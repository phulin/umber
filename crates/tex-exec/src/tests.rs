use super::*;
use tex_lex::{InputStack, MemoryInput, WorldInput};
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::meaning::{ExpandablePrimitive, Meaning};
use tex_state::token::{Catcode, OriginId, Token, TracedTokenWord};
use tex_state::{EffectRecord, ExpansionState, PrintSink};
use tex_state::{InteractionMode, Universe};

mod align;
mod assignments;
mod boxes;
mod core;
mod fonts;
mod grouping_parity;
mod groups;
mod hyphenation;

#[test]
fn paragraph_mutation_entry_class_distinguishes_root_from_live_groups() {
    assert!(crate::paragraph_memo::same_mutation_entry_class(false, 0));
    assert!(crate::paragraph_memo::same_mutation_entry_class(true, 1));
    assert!(crate::paragraph_memo::same_mutation_entry_class(true, 9));
    assert!(!crate::paragraph_memo::same_mutation_entry_class(false, 1));
    assert!(!crate::paragraph_memo::same_mutation_entry_class(true, 0));
}
mod io;
mod math;
pub(crate) mod support;
