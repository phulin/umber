//! Actual TeX glue pointer reassignment test.
use super::*;

#[allow(clippy::too_many_arguments)] // borrows actual semantic owners and concrete operands
pub(super) fn glue_pointer_reassigned<G>(
    context: &CommandContext<'_, G>,
    mu: bool,
    index: u16,
    value: GlueSpec,
    source_identity: Option<GlueId<G>>,
    source_register: Option<(bool, u16)>,
    global: bool,
    skip_pointer_sources: &[GluePointerSource<G>],
    muskip_pointer_sources: &[GluePointerSource<G>],
) -> bool {
    if global {
        return false;
    }
    let physical = if mu {
        context.muskip(index)
    } else {
        context.glue_register(index).ok().flatten()
    };
    let Some(physical) = physical else {
        return false;
    };
    if context.glue(physical) == GlueSpec::ZERO && value == GlueSpec::ZERO {
        return true;
    }
    let Some(source_identity) = source_identity else {
        return false;
    };
    if source_register == Some((mu, index)) {
        return true;
    }
    let pointer_sources = if mu {
        muskip_pointer_sources
    } else {
        skip_pointer_sources
    };
    let canonical_source = pointer_sources
        .get(usize::from(index))
        .and_then(|entry| *entry)
        .filter(|(recorded, _)| *recorded == physical)
        .map_or(Some(physical), |(_, source)| source);
    canonical_source == Some(source_identity)
}
