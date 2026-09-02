/// Exact safe-layer counters for focused dense-arena measurements.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ArenaMetrics {
    pub superblocks_allocated: u64,
    pub superblocks_reused: u64,
    pub superblocks_released: u64,
    pub blocks_truncated: u64,
    pub values_constructed: u64,
    pub values_truncated: u64,
    pub direct_lookups: u64,
    pub descriptor_visits: u64,
    pub cursor_captures: u64,
    pub logical_rows_created: u64,
    pub logical_rows_reused: u64,
    pub logical_rows_released: u64,
    pub logical_stale_rejections: u64,
    pub physical_stale_rejections: u64,
    pub forked_arenas: u64,
    pub fork_tail_values_copied: u64,
    pub fork_tail_bytes_copied: u64,
    pub table_entries_copied: u64,
    pub table_live_entries_copied: u64,
    pub table_vacant_entries_copied: u64,
    pub table_bytes_copied: u64,
    pub accepted_payload_copies: u64,
    pub rejected_payload_copies: u64,
    pub boundary_rotations: u64,
    pub boundary_slack_values: u64,
    pub block_ranges_detached: u64,
    pub block_ranges_prepared: u64,
    pub block_ranges_transferred: u64,
    pub block_ranges_rolled_back: u64,
}

impl ArenaMetrics {
    pub(crate) fn merged(self, other: Self) -> Self {
        Self {
            superblocks_allocated: self.superblocks_allocated + other.superblocks_allocated,
            superblocks_reused: self.superblocks_reused + other.superblocks_reused,
            superblocks_released: self.superblocks_released + other.superblocks_released,
            blocks_truncated: self.blocks_truncated + other.blocks_truncated,
            values_constructed: self.values_constructed + other.values_constructed,
            values_truncated: self.values_truncated + other.values_truncated,
            direct_lookups: self.direct_lookups + other.direct_lookups,
            descriptor_visits: self.descriptor_visits + other.descriptor_visits,
            cursor_captures: self.cursor_captures + other.cursor_captures,
            logical_rows_created: self.logical_rows_created + other.logical_rows_created,
            logical_rows_reused: self.logical_rows_reused + other.logical_rows_reused,
            logical_rows_released: self.logical_rows_released + other.logical_rows_released,
            logical_stale_rejections: self.logical_stale_rejections
                + other.logical_stale_rejections,
            physical_stale_rejections: self.physical_stale_rejections
                + other.physical_stale_rejections,
            forked_arenas: self.forked_arenas + other.forked_arenas,
            fork_tail_values_copied: self.fork_tail_values_copied + other.fork_tail_values_copied,
            fork_tail_bytes_copied: self.fork_tail_bytes_copied + other.fork_tail_bytes_copied,
            table_entries_copied: self.table_entries_copied + other.table_entries_copied,
            table_live_entries_copied: self.table_live_entries_copied
                + other.table_live_entries_copied,
            table_vacant_entries_copied: self.table_vacant_entries_copied
                + other.table_vacant_entries_copied,
            table_bytes_copied: self.table_bytes_copied + other.table_bytes_copied,
            accepted_payload_copies: self.accepted_payload_copies + other.accepted_payload_copies,
            rejected_payload_copies: self.rejected_payload_copies + other.rejected_payload_copies,
            boundary_rotations: self.boundary_rotations + other.boundary_rotations,
            boundary_slack_values: self.boundary_slack_values + other.boundary_slack_values,
            block_ranges_detached: self.block_ranges_detached + other.block_ranges_detached,
            block_ranges_prepared: self.block_ranges_prepared + other.block_ranges_prepared,
            block_ranges_transferred: self.block_ranges_transferred
                + other.block_ranges_transferred,
            block_ranges_rolled_back: self.block_ranges_rolled_back
                + other.block_ranges_rolled_back,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ForkShape {
    pub accepted_blocks: usize,
    pub candidate_blocks: usize,
    pub shared_complete_blocks: usize,
    pub candidate_private_blocks: usize,
}
