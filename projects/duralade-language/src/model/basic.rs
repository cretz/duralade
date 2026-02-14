use serde::{Deserialize, Serialize};

use super::{NodeId, Symbol};

/// Spec 4 - comment (consecutive comment lines grouped together)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Comment {
    pub node_id: NodeId,
    pub lines: Vec<CommentLine>,
}

/// A single line within a comment block
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommentLine {
    pub node_id: NodeId,
    pub text: String,
}

/// Spec 5 - identifier
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Ident {
    pub node_id: NodeId,
    pub name: Symbol,
    pub is_raw: bool,
}
