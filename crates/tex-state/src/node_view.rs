//! Borrowed views of compact page-material nodes.
//!
//! The page-material arena owns node storage. This module exposes only direct
//! borrowed traversal and node projections; page coordinates and ownership
//! operations live in `page_node_arena`.

mod cursor;
mod view;
pub use cursor::*;
pub use view::*;
