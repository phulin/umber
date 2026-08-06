//! Authoritative scoped assignment commits and typed mutation receipts.
//!
//! Scanners deliver typed values; this boundary alone decides whether an
//! e-TeX local write is redundant, selects the local/global state barrier,
//! emits `\tracingassigns`, and returns the mutation that became observable.

use tex_command::{MutationRecord, MutationTarget, ObservationValue};
use tex_state::Universe;
use tex_state::env::banks::{DimenParam, GlueParam, IntParam, TokParam};
use tex_state::glue::GlueSpec;
use tex_state::ids::{GlueId, NodeListId, TokenListId};
use tex_state::interner::Symbol;
use tex_state::meaning::Meaning;
use tex_state::scaled::Scaled;
use tex_state::token::Token;

use super::tracing;

/// The result of one authoritative assignment commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MutationReceipt(Option<MutationRecord>);

impl MutationReceipt {
    pub(crate) const SILENT: Self = Self(None);

    pub(crate) fn observed(record: MutationRecord) -> Self {
        Self(Some(record))
    }

    pub(crate) fn into_record(self) -> Option<MutationRecord> {
        self.0
    }
}

/// Borrow-scoped authority for all state written by TeX assignment families.
pub(crate) struct AssignmentCommitter<'a> {
    stores: &'a mut Universe,
}

impl<'a> AssignmentCommitter<'a> {
    pub(crate) fn new(stores: &'a mut Universe) -> Self {
        Self { stores }
    }

    fn redundant_word<T: Eq>(&self, current: T, replacement: T) -> bool {
        self.stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0 && current == replacement
    }

    fn redundant_zero_glue(&self, current: GlueId, replacement: &GlueSpec) -> bool {
        self.stores.int_param(IntParam::ETEX_EXTENDED_MODE) > 0
            && current == GlueId::ZERO
            && *replacement == GlueSpec::ZERO
    }

    pub(crate) fn scoped_word<T, Write, Trace>(
        &mut self,
        current: T,
        replacement: T,
        global: bool,
        record: MutationRecord,
        write: Write,
        trace: Trace,
    ) -> MutationReceipt
    where
        T: Eq + Copy,
        Write: FnOnce(&mut Universe, bool),
        Trace: FnOnce(&mut Universe, bool),
    {
        let redundant = !global && self.redundant_word(current, replacement);
        if global || !redundant {
            write(self.stores, global);
        }
        trace(self.stores, !redundant);
        if redundant {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(record)
        }
    }

    pub(crate) fn unscoped<Write>(
        &mut self,
        record: Option<MutationRecord>,
        write: Write,
    ) -> MutationReceipt
    where
        Write: FnOnce(&mut Universe),
    {
        write(self.stores);
        MutationReceipt(record)
    }

    pub(crate) fn try_unscoped<E, Write>(
        &mut self,
        record: Option<MutationRecord>,
        write: Write,
    ) -> Result<MutationReceipt, E>
    where
        Write: FnOnce(&mut Universe) -> Result<(), E>,
    {
        write(self.stores)?;
        Ok(MutationReceipt(record))
    }

    pub(crate) fn count(&mut self, index: u16, value: i32, global: bool) -> MutationReceipt {
        let old = self.stores.count(index);
        let redundant = !global && self.redundant_word(old, value);
        if global {
            self.stores.set_count_global(index, value);
        } else if !redundant {
            self.stores.set_count(index, value);
        }
        tracing::trace_int_register(self.stores, index, global, old, value);
        if redundant && index <= 255 {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!("count:{index}")),
                value: ObservationValue::Integer(i64::from(value)),
                global,
            })
        }
    }

    pub(crate) fn dimension(&mut self, index: u16, value: Scaled, global: bool) -> MutationReceipt {
        let old = self.stores.dimen(index);
        let redundant = !global && self.redundant_word(old, value);
        if global {
            self.stores.set_dimen_global(index, value);
        } else if !redundant {
            self.stores.set_dimen(index, value);
        }
        tracing::trace_dimen_register(self.stores, index, global, old, value);
        if redundant && index <= 255 {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!("dimen:{index}")),
                value: ObservationValue::Scaled(i64::from(value.raw())),
                global,
            })
        }
    }

    pub(crate) fn skip(
        &mut self,
        index: u16,
        value: GlueSpec,
        global: bool,
        mu: bool,
        redundant: bool,
        reassigning: bool,
    ) -> MutationReceipt {
        let old = if mu {
            self.stores.muskip(index)
        } else {
            self.stores.skip(index)
        };
        let redundant = !global && (redundant || self.redundant_zero_glue(old, &value));
        let new = if redundant {
            old
        } else {
            let new = self.stores.intern_glue(value);
            match (mu, global) {
                (true, true) => self.stores.set_muskip_global(index, new),
                (true, false) => self.stores.set_muskip(index, new),
                (false, true) => self.stores.set_skip_global(index, new),
                (false, false) => self.stores.set_skip(index, new),
            }
            new
        };
        let changed = !(reassigning
            || (self.stores.glue(old) == GlueSpec::ZERO
                && self.stores.glue(new) == GlueSpec::ZERO));
        if mu {
            tracing::trace_muglue_register(self.stores, index, global, old, new, changed);
        } else {
            tracing::trace_glue_register(self.stores, index, global, old, new, changed);
        }
        if redundant && index <= 255 {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!(
                    "{}:{index}",
                    if mu { "muskip" } else { "skip" }
                )),
                value: glue_value(&value),
                global,
            })
        }
    }

    pub(crate) fn toks(
        &mut self,
        index: u16,
        value: TokenListId,
        observed: ObservationValue,
        global: bool,
    ) -> MutationReceipt {
        let old = self.stores.toks(index);
        let redundant = !global && self.redundant_word(old, value);
        if global {
            self.stores.set_toks_global(index, value);
        } else if !redundant {
            self.stores.set_toks(index, value);
        }
        tracing::trace_toks_register(self.stores, index, global, old, value);
        if redundant && index <= 255 {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!("toks:{index}")),
                value: observed,
                global,
            })
        }
    }

    pub(crate) fn int_parameter(
        &mut self,
        index: u16,
        value: i32,
        key: String,
        global: bool,
    ) -> MutationReceipt {
        let parameter = IntParam::new(index);
        let old = self.stores.int_param(parameter);
        let tracing_before = self.stores.int_param(IntParam::TRACING_ASSIGNS) > 0;
        let redundant = !global && self.redundant_word(old, value);
        if global {
            self.stores.set_int_param_global(parameter, value);
        } else if !redundant {
            self.stores.set_int_param(parameter, value);
        }
        tracing::trace_int_param(self.stores, index, tracing_before, global, old, value);
        if redundant {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Parameter,
                key: ObservationValue::Name(key),
                value: ObservationValue::Integer(i64::from(value)),
                global,
            })
        }
    }

    pub(crate) fn dimension_parameter(
        &mut self,
        index: u16,
        value: Scaled,
        key: String,
        global: bool,
    ) -> MutationReceipt {
        let parameter = DimenParam::new(index);
        let old = self.stores.dimen_param(parameter);
        let redundant = !global && self.redundant_word(old, value);
        if global {
            self.stores.set_dimen_param_global(parameter, value);
        } else if !redundant {
            self.stores.set_dimen_param(parameter, value);
        }
        tracing::trace_dimen_param(self.stores, index, global, old, value);
        if redundant {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Parameter,
                key: ObservationValue::Name(key),
                value: ObservationValue::Scaled(i64::from(value.raw())),
                global,
            })
        }
    }

    pub(crate) fn token_parameter(
        &mut self,
        index: u16,
        value: TokenListId,
        observed: ObservationValue,
        key: String,
        global: bool,
    ) -> MutationReceipt {
        let parameter = TokParam::new(index);
        let old = self.stores.tok_param(parameter);
        let redundant = !global && self.redundant_word(old, value);
        if global {
            self.stores.set_tok_param_global(parameter, value);
        } else if !redundant {
            self.stores.set_tok_param(parameter, value);
        }
        tracing::trace_tok_param(self.stores, index, global, old, value);
        if redundant {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Parameter,
                key: ObservationValue::Name(key),
                value: observed,
                global,
            })
        }
    }

    pub(crate) fn glue_parameter(
        &mut self,
        index: u16,
        value: GlueSpec,
        key: String,
        global: bool,
    ) -> MutationReceipt {
        let parameter = GlueParam::new(index);
        let old = self.stores.glue_param(parameter);
        let redundant = !global && self.redundant_zero_glue(old, &value);
        let new = if redundant {
            old
        } else {
            let new = self.stores.intern_glue(value);
            if global {
                self.stores.set_glue_param_global(parameter, new);
            } else {
                self.stores.set_glue_param(parameter, new);
            }
            new
        };
        tracing::trace_glue_param(self.stores, index, global, old, new, !redundant);
        if redundant {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Parameter,
                key: ObservationValue::Name(key),
                value: glue_value(&value),
                global,
            })
        }
    }

    pub(crate) fn meaning<F>(
        &mut self,
        target: Symbol,
        token: Token,
        meaning: Meaning,
        observed: ObservationValue,
        global: bool,
        write: F,
    ) -> MutationReceipt
    where
        F: FnOnce(&mut Universe),
    {
        let redundant = !global && self.redundant_word(self.stores.meaning(target), meaning);
        tracing::trace_meaning_write(self.stores, token, !redundant, global, |stores| {
            if global || !redundant {
                write(stores);
            }
        });
        if redundant {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Meaning,
                key: ObservationValue::Name(self.stores.resolve(target).to_owned()),
                value: observed,
                global,
            })
        }
    }

    pub(crate) fn box_register<F>(
        &mut self,
        index: u16,
        boxed: Option<NodeListId>,
        global: bool,
        write: F,
    ) -> MutationReceipt
    where
        F: FnOnce(&mut Universe),
    {
        tracing::trace_box_write(self.stores, index, global, boxed, write);
        if index <= 255 {
            MutationReceipt::SILENT
        } else {
            MutationReceipt::observed(MutationRecord {
                target: MutationTarget::Register,
                key: ObservationValue::Name(format!("box:{index}")),
                value: ObservationValue::Name(
                    if boxed.is_some() { "occupied" } else { "void" }.into(),
                ),
                global,
            })
        }
    }
}

fn glue_value(value: &GlueSpec) -> ObservationValue {
    ObservationValue::Glue {
        width: i64::from(value.width.raw()),
        stretch: i64::from(value.stretch.raw()),
        stretch_order: tex_command::canonical_names::glue_order_name(value.stretch_order),
        shrink: i64::from(value.shrink.raw()),
        shrink_order: tex_command::canonical_names::glue_order_name(value.shrink_order),
    }
}
