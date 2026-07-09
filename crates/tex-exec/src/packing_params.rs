//! Execution-side snapshots of packing parameters.

use tex_state::Universe;
use tex_state::env::banks::{DimenParam, IntParam};
use tex_state::ids::NodeListId;
use tex_typeset::{HpackParams, PackSpec, PackedBox, VpackParams};

#[must_use]
pub(crate) fn hpack_params(stores: &Universe) -> HpackParams {
    HpackParams {
        hbadness: stores.int_param(IntParam::HBADNESS),
        hfuzz: stores.dimen_param(DimenParam::HFUZZ),
        overfull_rule: stores.dimen_param(DimenParam::OVERFULL_RULE),
    }
}

#[must_use]
pub(crate) fn vpack_params(stores: &Universe) -> VpackParams {
    VpackParams {
        vbadness: stores.int_param(IntParam::VBADNESS),
        vfuzz: stores.dimen_param(DimenParam::VFUZZ),
        box_max_depth: stores.dimen_param(DimenParam::BOX_MAX_DEPTH),
    }
}

pub(crate) fn hpack(
    stores: &mut Universe,
    list: NodeListId,
    spec: PackSpec,
    params: HpackParams,
) -> PackedBox {
    let packed = tex_typeset::hpack(&*stores, list, spec, params);
    stores.set_last_badness(packed.badness);
    packed
}

pub(crate) fn vpack(
    stores: &mut Universe,
    list: NodeListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let packed = tex_typeset::vpack(&*stores, list, spec, params);
    stores.set_last_badness(packed.badness);
    packed
}

pub(crate) fn vtop(
    stores: &mut Universe,
    list: NodeListId,
    spec: PackSpec,
    params: VpackParams,
) -> PackedBox {
    let packed = tex_typeset::vtop(&*stores, list, spec, params);
    stores.set_last_badness(packed.badness);
    packed
}
