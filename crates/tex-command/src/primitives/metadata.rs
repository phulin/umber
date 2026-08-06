//! Declarative primitive spelling, meaning, profile, and observation metadata.

use crate::CommandDialect;
use tex_state::meaning::{ExpandablePrimitive, Meaning, UnexpandablePrimitive};

use crate::observation::LOCAL_BASE;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum PrimitiveSet {
    Tex82,
    Etex,
    Latex,
    Pdftex,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct PrimitiveSpelling {
    pub(crate) set: PrimitiveSet,
    pub(crate) name: &'static str,
    pub(crate) install_in_initex: bool,
    pub(crate) register_after_format_load: bool,
}

impl PrimitiveSpelling {
    pub(crate) const fn installation(self) -> super::InstallationPolicy {
        match (self.install_in_initex, self.register_after_format_load) {
            (true, true) => super::InstallationPolicy::BOTH,
            (true, false) => super::InstallationPolicy::INITEX,
            (false, true) => super::InstallationPolicy::FORMAT_REGISTRY,
            (false, false) => super::InstallationPolicy::NONE,
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct PrimitiveMetadata {
    pub(crate) meaning: Meaning,
    pub(crate) spellings: &'static [PrimitiveSpelling],
}

const VMODE: i64 = 1;
const HMODE: i64 = VMODE + 100 + 1;
const WIDTH_OFFSET: i64 = 1;
const HEIGHT_OFFSET: i64 = 3;
const DEPTH_OFFSET: i64 = 2;

const fn def_code_base(dialect: CommandDialect) -> i64 {
    match dialect {
        CommandDialect::Etex26 => 25_636,
        CommandDialect::Tex82 | CommandDialect::Pdftex14029 => 25_631,
    }
}

const fn del_code_base(dialect: CommandDialect) -> i64 {
    match dialect {
        CommandDialect::Etex26 => 27_501,
        CommandDialect::Tex82 | CommandDialect::Pdftex14029 => 27_485,
    }
}

const fn math_font_base(dialect: CommandDialect) -> i64 {
    match dialect {
        CommandDialect::Etex26 => 25_588,
        CommandDialect::Tex82 | CommandDialect::Pdftex14029 => 25_583,
    }
}

const fn penalty_array_base(dialect: CommandDialect) -> i64 {
    match dialect {
        CommandDialect::Tex82 | CommandDialect::Etex26 => 25_324,
        CommandDialect::Pdftex14029 => 25_328,
    }
}

macro_rules! primitive_metadata {
    (
        expandable($expandable_dialect:ident) {
            $(
                $expandable:ident => {
                    spellings: [$(($expandable_set:ident, $expandable_name:literal)),* $(,)?],
                    identity: ($expandable_command:literal, $expandable_operand:expr)
                }
            ),* $(,)?
        }
        unexpandable($unexpandable_dialect:ident) {
            $(
                $unexpandable:ident => {
                    spellings: [$(($unexpandable_set:ident, $unexpandable_name:literal)),* $(,)?],
                    identity: ($unexpandable_command:literal, $unexpandable_operand:expr)
                }
            ),* $(,)?
        }
    ) => {
        pub(crate) const EXPANDABLE_PRIMITIVES: &[PrimitiveMetadata] = &[
            $(PrimitiveMetadata {
                meaning: Meaning::ExpandablePrimitive(ExpandablePrimitive::$expandable),
                spellings: &[$(PrimitiveSpelling {
                    set: PrimitiveSet::$expandable_set,
                    name: $expandable_name,
                    install_in_initex: true,
                    register_after_format_load: true,
                }),*],
            }),*
        ];

        pub(crate) const UNEXPANDABLE_PRIMITIVES: &[PrimitiveMetadata] = &[
            $(PrimitiveMetadata {
                meaning: Meaning::UnexpandablePrimitive(UnexpandablePrimitive::$unexpandable),
                spellings: &[$(PrimitiveSpelling {
                    set: PrimitiveSet::$unexpandable_set,
                    name: $unexpandable_name,
                    install_in_initex: true,
                    register_after_format_load: true,
                }),*],
            }),*
        ];

        pub(crate) fn expandable_identity(
            $expandable_dialect: CommandDialect,
            primitive: ExpandablePrimitive,
        ) -> (&'static str, Option<i64>) {
            match primitive {
                $(ExpandablePrimitive::$expandable => {
                    ($expandable_command, $expandable_operand)
                }),*
            }
        }

        pub(crate) fn unexpandable_identity(
            $unexpandable_dialect: CommandDialect,
            primitive: UnexpandablePrimitive,
        ) -> (&'static str, Option<i64>) {
            match primitive {
                $(UnexpandablePrimitive::$unexpandable => {
                    ($unexpandable_command, $unexpandable_operand)
                }),*
            }
        }
    };
}

include!("primitive_metadata.rs");
