use std::collections::HashMap;
use tex_state::env::Env;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::ids::{GlueId, NodeListId, TokenListId};
use tex_state::interner::Symbol;
use tex_state::meaning::Meaning;
use tex_state::scaled::Scaled;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TestCell {
    Meaning(u32),
    Count(u16),
    Dimen(u16),
    Skip(u16),
    Toks(u16),
    Box(u16),
    IntParam(u16),
    DimenParam(u16),
    GlueParam(u16),
    TokParam(u16),
}

impl TestCell {
    pub(crate) fn set(self, env: &mut Env, word: u64, global: bool) {
        match self {
            Self::Meaning(index) => {
                let value = meaning(word);
                if global {
                    env.set_global(Symbol::testing_new(index), value);
                } else {
                    env.set(Symbol::testing_new(index), value);
                }
            }
            Self::Count(index) => {
                let value = word as u32 as i32;
                if global {
                    env.set_count_global(index, value);
                } else {
                    env.set_count(index, value);
                }
            }
            Self::Dimen(index) => {
                let value = Scaled::from_raw(word as u32 as i32);
                if global {
                    env.set_dimen_global(index, value);
                } else {
                    env.set_dimen(index, value);
                }
            }
            Self::Skip(index) => {
                let value = GlueId::testing_new(word as u32);
                if global {
                    env.set_skip_global(index, value);
                } else {
                    env.set_skip(index, value);
                }
            }
            Self::Toks(index) => {
                let value = TokenListId::testing_new(word as u32);
                if global {
                    env.set_toks_global(index, value);
                } else {
                    env.set_toks(index, value);
                }
            }
            Self::Box(index) => {
                let value = NodeListId::testing_new(word as u32);
                if global {
                    env.set_box_reg_global(index, value);
                } else {
                    env.set_box_reg(index, value);
                }
            }
            Self::IntParam(index) => {
                let value = word as u32 as i32;
                if global {
                    env.set_int_param_global(IntParam::new(index), value);
                } else {
                    env.set_int_param(IntParam::new(index), value);
                }
            }
            Self::DimenParam(index) => {
                let value = Scaled::from_raw(word as u32 as i32);
                if global {
                    env.set_dimen_param_global(DimenParam::new(index), value);
                } else {
                    env.set_dimen_param(DimenParam::new(index), value);
                }
            }
            Self::GlueParam(index) => {
                let value = GlueId::testing_new(word as u32);
                if global {
                    env.set_glue_param_global(GlueParam::new(index), value);
                } else {
                    env.set_glue_param(GlueParam::new(index), value);
                }
            }
            Self::TokParam(index) => {
                let value = TokenListId::testing_new(word as u32);
                if global {
                    env.set_tok_param_global(TokParam::new(index), value);
                } else {
                    env.set_tok_param(TokParam::new(index), value);
                }
            }
        }
    }

    pub(crate) fn get(self, env: &Env) -> u64 {
        match self {
            Self::Meaning(index) => env.get(Symbol::testing_new(index)).encode(),
            Self::Count(index) => u64::from(env.count(index) as u32),
            Self::Dimen(index) => u64::from(env.dimen(index).raw() as u32),
            Self::Skip(index) => u64::from(env.skip(index).raw()),
            Self::Toks(index) => u64::from(env.toks(index).raw()),
            Self::Box(index) => u64::from(env.box_reg(index).raw()),
            Self::IntParam(index) => u64::from(env.int_param(IntParam::new(index)) as u32),
            Self::DimenParam(index) => {
                u64::from(env.dimen_param(DimenParam::new(index)).raw() as u32)
            }
            Self::GlueParam(index) => u64::from(env.glue_param(GlueParam::new(index)).raw()),
            Self::TokParam(index) => u64::from(env.tok_param(TokParam::new(index)).raw()),
        }
    }
}

#[derive(Debug)]
pub(crate) struct Oracle {
    scopes: Vec<HashMap<TestCell, u64>>,
}

impl Oracle {
    pub(crate) fn new() -> Self {
        Self {
            scopes: vec![HashMap::new()],
        }
    }

    pub(crate) fn enter_group(&mut self) {
        self.scopes.push(HashMap::new());
    }

    pub(crate) fn leave_group(&mut self) {
        assert!(self.scopes.len() > 1, "oracle group underflow");
        self.scopes.pop();
    }

    pub(crate) fn set(&mut self, cell: TestCell, word: u64, global: bool) {
        let word = canonical_word(cell, word);
        if global {
            for scope in &mut self.scopes {
                set_word(scope, cell, word);
            }
        } else {
            let scope = self.scopes.last_mut().expect("oracle has a root scope");
            set_word(scope, cell, word);
        }
    }

    pub(crate) fn assert_matches(&self, env: &Env, cells: &[TestCell]) {
        for &cell in cells {
            assert_eq!(cell.get(env), self.get(cell), "oracle mismatch at {cell:?}");
        }
    }

    fn get(&self, cell: TestCell) -> u64 {
        self.scopes
            .iter()
            .rev()
            .find_map(|scope| scope.get(&cell).copied())
            .unwrap_or(0)
    }
}

fn set_word(scope: &mut HashMap<TestCell, u64>, cell: TestCell, word: u64) {
    scope.insert(cell, word);
}

fn canonical_word(cell: TestCell, word: u64) -> u64 {
    match cell {
        TestCell::Meaning(_) => meaning(word).encode(),
        _ => u64::from(word as u32),
    }
}

fn meaning(word: u64) -> Meaning {
    match word % 4 {
        0 => Meaning::Undefined,
        1 => Meaning::Relax,
        2 => Meaning::CharGiven(char::from_u32(32 + (word as u32 % 95)).expect("ASCII graphic")),
        _ => Meaning::Raw {
            op: 200,
            operand: word & ((1_u64 << 48) - 1),
        },
    }
}
