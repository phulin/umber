//! TeX82 §§1230--1234 code-table value recovery shared by hot and cold assignments.

use super::*;

pub(super) fn recover_code_table_value<G>(
    primitive: UnexpandablePrimitive,
    raw_value: i32,
    stores: &mut tex_state::CommandContext<'_, G>,
    command: &mut CommandMachine<'_, '_, G>,
) -> Result<i32, ExecError> {
    let maximum = match primitive {
        UnexpandablePrimitive::CatCode => 15,
        UnexpandablePrimitive::LcCode | UnexpandablePrimitive::UcCode => 255,
        UnexpandablePrimitive::SfCode => 32_767,
        UnexpandablePrimitive::MathCode => 32_768,
        UnexpandablePrimitive::DelCode => 0xFF_FFFF,
        _ => unreachable!("only code-table primitives reach value recovery"),
    };
    let valid = (0..=maximum).contains(&raw_value)
        || (primitive == UnexpandablePrimitive::DelCode && raw_value == -1);
    if valid {
        return Ok(raw_value);
    }

    // The scanner has consumed the complete operand. §1230 reports the bad
    // value, substitutes zero, then commits that assignment in this episode.
    let context = command.state.output_open_context(stores);
    let mut report = stores.print_err("Invalid code (");
    report
        .print_int(raw_value)
        .print("), should be in the range 0..")
        .print_int(maximum)
        .help(&["I changed this one to zero."])
        .context(context);
    report.error().defer_recovery(command.diagnostic_effects)?;
    Ok(0)
}
