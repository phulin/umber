use tex_command::{ParameterBankClass, PrimitiveProfile, primitive_parameter_views};
use tex_state::Universe;

/// Installs TeX82's complete non-expandable catalogue layer.
pub fn install_unexpandable_primitives<G>(stores: &mut Universe<G>) {
    tex_command::install_tex82_unexpandable_primitives(stores);
}

/// Reconstructs TeX82's primitive registry without shadowing format meanings.
pub fn register_unexpandable_primitives<G>(stores: &mut Universe<G>) {
    tex_command::register_tex82_unexpandable_primitives(stores);
}

/// Installs the complete e-TeX non-expandable catalogue layer.
pub fn install_etex_unexpandable_primitives<G>(stores: &mut Universe<G>) {
    tex_command::install_etex_unexpandable_primitives(stores);
}

/// Reconstructs the e-TeX registry without shadowing format meanings.
pub fn register_etex_unexpandable_primitives<G>(stores: &mut Universe<G>) {
    tex_command::register_etex_unexpandable_primitives(stores);
}

/// e-TeX `\tracingassigns`'s `show_eqtb`-equivalent parameter name.
pub(crate) fn int_param_name(index: u16) -> String {
    parameter_name(ParameterBankClass::Integer, index, "IntParam")
}

pub(crate) fn dimen_param_name(index: u16) -> String {
    parameter_name(ParameterBankClass::Dimension, index, "DimenParam")
}

pub(crate) fn tok_param_name(index: u16) -> String {
    parameter_name(ParameterBankClass::Tokens, index, "TokParam")
}

/// Looks up a glue parameter's name and TeX display unit.
pub(crate) fn glue_param_name(index: u16) -> (String, &'static str) {
    for profile in [PrimitiveProfile::Tex82, PrimitiveProfile::Etex26] {
        if let Some(row) = primitive_parameter_views(profile).into_iter().find(|row| {
            row.cell.index == index
                && matches!(
                    row.cell.class,
                    ParameterBankClass::Glue | ParameterBankClass::MathGlue
                )
        }) {
            let unit = if row.cell.class == ParameterBankClass::MathGlue {
                "mu"
            } else {
                "pt"
            };
            return (row.name.to_owned(), unit);
        }
    }
    (format!("GlueParam{index}"), "pt")
}

fn parameter_name(class: ParameterBankClass, index: u16, fallback: &str) -> String {
    for profile in [PrimitiveProfile::Tex82, PrimitiveProfile::Etex26] {
        if let Some(row) = primitive_parameter_views(profile)
            .into_iter()
            .find(|row| row.cell.class == class && row.cell.index == index)
        {
            return row.name.to_owned();
        }
    }
    format!("{fallback}{index}")
}
