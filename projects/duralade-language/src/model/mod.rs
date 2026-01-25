use serde::{Deserialize, Serialize};

// TODO: Add custom Serialize impl to conditionally skip serialization based on thread-local
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidNode {
    pub node_id: NodeId,
    // TODO: Add message, expected, found, recovered_at, etc. for richer diagnostics
}

mod basic;
mod construct;
mod expr;
mod source;
mod stmt;
mod types;

pub use basic::*;
pub use construct::*;
pub use expr::*;
pub use source::*;
pub use stmt::*;
pub use types::*;
