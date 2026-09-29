//! One-proof transfer of a closed construction suffix between regions.

use super::*;

impl<Role> NodeRegion<Role> {
    /// Seals the construction suffix opened at `mark` and moves it, with its
    /// paired annex suffix, into `destination`.
    ///
    /// Both lanes are proved once, then each chunk is rebranded in one
    /// metadata write; no detached intermediate loan exists. Every failure precedes mutation and returns the live
    /// build authority, so an `InvalidRegion` rejection may still select a
    /// structural copy.
    #[allow(clippy::result_large_err)] // Failure returns the sole move-only build authority.
    pub(crate) fn move_closure_suffix_into<Destination>(
        &mut self,
        pool: &mut NodePool,
        mark: ClosureBuildMark<Role>,
        root: RegionRoot<Role>,
        receipt: ConsumedClosureRootsReceipt<Role>,
        destination: &mut NodeRegion<Destination>,
    ) -> Result<RegionRoot<Destination>, ClosureSealError<Role>> {
        let node_start = mark.batch.payload_start();
        let annex_start = mark.annex_batch.payload_start();
        let preflight = pool
            .validate_region(self)
            .and_then(|()| pool.validate_region(destination))
            .and_then(|()| {
                if mark.region != self.id
                    || receipt.region != self.id
                    || receipt.serial != mark.serial
                    || root.region != self.id
                {
                    return Err(ForkArenaError::InvalidRegion);
                }
                self.pub_arena.preflight_suffix_move(
                    &pool.chunks,
                    &destination.pub_arena,
                    node_start,
                    &[root.list.coordinate()],
                    Some(annex_start),
                )?;
                self.annex_arena.preflight_suffix_move(
                    &pool.annex_chunks,
                    &destination.annex_arena,
                    annex_start,
                    &[],
                    None,
                )
            });
        if let Err(error) = preflight {
            return Err(ClosureSealError { error, mark });
        }
        // Moved records may name borrowed bodies; logging every source
        // entry keeps the destination's log a superset.
        pool.inherit_borrows(self, destination);
        let destination_annex_start = destination.annex_arena.live_payload_chunks();
        self.pub_arena.move_proved_suffix_into(
            &mut pool.chunks,
            &mut destination.pub_arena,
            node_start,
            SuffixFloorRebase {
                nodes: false,
                paired: Some(PairedFloorRebase {
                    source: annex_start,
                    destination: destination_annex_start,
                }),
            },
        );
        self.annex_arena.move_proved_suffix_into(
            &mut pool.annex_chunks,
            &mut destination.annex_arena,
            annex_start,
            SuffixFloorRebase {
                nodes: false,
                paired: None,
            },
        );
        pool.closure_transitions.envelope_moves =
            pool.closure_transitions.envelope_moves.saturating_add(1);
        Ok(RegionRoot {
            region: destination.id,
            list: root.list,
            _role: PhantomData,
        })
    }
}
