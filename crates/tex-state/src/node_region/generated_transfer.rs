//! Prepared generated inline-body transfer and exact inverse receipts.

use super::*;

pub(crate) enum GeneratedInlinePiece {
    Full(PageListId),
    Cut(crate::page_node_arena::PageBoxCutRange),
}

pub(super) struct GeneratedInlineHead {
    pub(super) piece: usize,
    pub(super) loan: ConsumedHeadEdgeLoan<PageMaterialLane>,
    pub(super) bound_prefix: Option<crate::fork_arena::ArenaListId<PageMaterialLane>>,
}

/// Projects cut direct records and moves complete chunks of one authenticated
/// generated inline chain. Destination cut chunks occupy their original
/// logical positions; complete source chunks fill the vacant positions
/// between them. The receipt reverses every edge, floor, and hole mutation.
pub(crate) fn transfer_page_generated_inline_selected(
    pool: &mut NodePool,
    source: &mut NodeRegion<PageRole>,
    pieces: &[GeneratedInlinePiece],
    node_ranges: &[std::ops::Range<usize>],
    semantic_identity_enabled: bool,
    destination: &mut NodeRegion<DurableRole>,
) -> Result<
    (
        crate::fork_arena::ArenaListId<PageMaterialLane>,
        PageInteriorTransferLoan,
    ),
    ForkArenaError,
> {
    pool.validate_region(source)?;
    pool.validate_region(destination)?;
    let empty = loan_empty_page_box_body(pool, source, 0, 0, destination)?;
    let PageInteriorTransferredChunks::Partitioned {
        nodes: empty_nodes,
        annex: empty_annex,
    } = empty.chunks
    else {
        unreachable!("empty paired loan is partitioned")
    };
    let mut loan = PageInteriorTransferLoan {
        source: empty.source,
        destination: empty.destination,
        chunks: PageInteriorTransferredChunks::GeneratedSelected {
            empty_nodes,
            empty_annex,
            holes: None,
            heads: Vec::new(),
            floors: None,
        },
    };
    let projected = (|| {
        let mut cuts = vec![None; pieces.len()];
        for (index, piece) in pieces.iter().enumerate() {
            let GeneratedInlinePiece::Cut(cut) = piece else {
                continue;
            };
            destination
                .pub_arena
                .reserve_vacant_prefix_until(&pool.chunks, cut.chunk_position)?;
            cuts[index] = Some(copy_consumed_direct_cut_into(
                pool,
                source,
                cut.chunk_position,
                cut.local.clone(),
                destination,
                semantic_identity_enabled,
                |_| None,
            )?);
        }
        if let Some(end) = node_ranges.last().map(|range| range.end)
            && end > destination.pub_arena.payload_position_end()
        {
            destination
                .pub_arena
                .reserve_vacant_prefix_until(&pool.chunks, end)?;
        }
        if !node_ranges.is_empty() {
            let PageInteriorTransferredChunks::GeneratedSelected {
                floors,
                heads,
                holes,
                ..
            } = &mut loan.chunks
            else {
                unreachable!()
            };
            *floors = Some(
                source
                    .pub_arena
                    .detach_consumed_inline_floors(&mut pool.chunks, node_ranges)?,
            );
            for (index, piece) in pieces.iter().enumerate() {
                let GeneratedInlinePiece::Full(root) = piece else {
                    continue;
                };
                heads.push(GeneratedInlineHead {
                    piece: index,
                    loan: source.pub_arena.detach_consumed_head_edge(
                        &mut pool.chunks,
                        root.coordinate(),
                        usize::MAX,
                    )?,
                    bound_prefix: None,
                });
            }
            *holes = Some(source.pub_arena.transfer_interior_into_holes(
                &mut pool.chunks,
                &mut destination.pub_arena,
                node_ranges,
            )?);
            pool.closure_transitions.envelope_moves =
                pool.closure_transitions.envelope_moves.saturating_add(1);
        }
        let mut joined = crate::fork_arena::ArenaListId::empty();
        let mut next_full_head = 0;
        for (index, piece) in pieces.iter().enumerate() {
            let coordinate = match piece {
                GeneratedInlinePiece::Full(root) => root.coordinate(),
                GeneratedInlinePiece::Cut(_) => cuts[index]
                    .ok_or(ForkArenaError::InvalidRegion)?
                    .coordinate(),
            };
            let prefix = joined;
            let unique = destination
                .pub_arena
                .reclaim_unlinked_validated_list(&pool.chunks, coordinate)?;
            joined = destination.pub_arena.append_unique_to_validated_list(
                &mut pool.chunks,
                joined,
                unique,
            )?;
            if matches!(piece, GeneratedInlinePiece::Full(_)) && !prefix.is_empty() {
                let PageInteriorTransferredChunks::GeneratedSelected { heads, .. } =
                    &mut loan.chunks
                else {
                    unreachable!()
                };
                let head = heads
                    .get_mut(next_full_head)
                    .ok_or(ForkArenaError::InvalidRegion)?;
                if head.piece != index {
                    return Err(ForkArenaError::InvalidRegion);
                }
                head.bound_prefix = Some(prefix);
            }
            if matches!(piece, GeneratedInlinePiece::Full(_)) {
                next_full_head += 1;
            }
        }
        Ok::<_, ForkArenaError>(joined)
    })();
    match projected {
        Ok(root) => Ok((root, loan)),
        Err(error) => {
            rollback_page_interior_closure(pool, source, destination, loan)
                .expect("failed generated projection returns every staged receipt");
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn rollback_generated_selected(
    pool: &mut NodePool,
    source: &mut NodeRegion<PageRole>,
    destination: &mut NodeRegion<DurableRole>,
    empty_nodes: TransferredIntervals<PageMaterialLane>,
    empty_annex: TransferredIntervals<NodeAnnexLane>,
    holes: Option<TransferredHoleIntervals<PageMaterialLane>>,
    heads: Vec<GeneratedInlineHead>,
    floors: Option<ConsumedInlineFloorLoan<PageMaterialLane>>,
) -> Result<(), ForkArenaError> {
    source.pub_arena.preflight_rollback_interior_intervals(
        &pool.chunks,
        &destination.pub_arena,
        &empty_nodes,
    )?;
    source.annex_arena.preflight_rollback_interior_intervals(
        &pool.annex_chunks,
        &destination.annex_arena,
        &empty_annex,
    )?;
    // A rejected forward hole preflight leaves the detached floors and heads
    // in their source owner. Only a committed hole loan moves them to the
    // destination. Check every inverse condition before restoring either
    // owner, including the empty paired lane receipt.
    if let Some(holes) = &holes {
        source.pub_arena.preflight_rollback_interior_holes(
            &pool.chunks,
            &destination.pub_arena,
            holes,
        )?;
        if let Some(floors) = &floors {
            destination
                .pub_arena
                .preflight_consumed_inline_floor_inverse(&pool.chunks, floors)?;
        }
        for head in &heads {
            destination.pub_arena.preflight_consumed_head_inverse(
                &pool.chunks,
                &head.loan,
                head.bound_prefix,
            )?;
        }
    } else {
        if let Some(floors) = &floors {
            source
                .pub_arena
                .preflight_consumed_inline_floor_inverse(&pool.chunks, floors)?;
        }
        for head in &heads {
            source
                .pub_arena
                .preflight_consumed_head_inverse(&pool.chunks, &head.loan, None)?;
        }
    }
    if holes.is_some() {
        for head in heads.iter().rev() {
            if let Some(prefix) = head.bound_prefix {
                destination.pub_arena.unbind_consumed_head_from_prefix(
                    &mut pool.chunks,
                    &head.loan,
                    prefix,
                )?;
            }
        }
    }
    if let Some(holes) = holes {
        source.pub_arena.rollback_interior_holes(
            &mut pool.chunks,
            &mut destination.pub_arena,
            holes,
        )?;
    }
    for head in heads.into_iter().rev() {
        source
            .pub_arena
            .restore_consumed_head_edge(&mut pool.chunks, head.loan)?;
    }
    if let Some(floors) = floors {
        source
            .pub_arena
            .restore_consumed_inline_floors(&mut pool.chunks, floors)?;
    }
    source.pub_arena.rollback_interior_intervals(
        &mut pool.chunks,
        &mut destination.pub_arena,
        empty_nodes,
    )?;
    source.annex_arena.rollback_interior_intervals(
        &mut pool.annex_chunks,
        &mut destination.annex_arena,
        empty_annex,
    )
}
