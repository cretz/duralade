use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{Comment, Construct, ExprInvocation, Ident, NodeId};

/// Position data for AST nodes, kept separate from the AST itself so it can
/// be stripped for serialization and runtime evaluation.
#[derive(Debug, Clone, Default)]
pub struct SourceMap {
    /// Maps each NodeId to its `(start, end)` byte range (end-exclusive).
    pub node_ranges: HashMap<NodeId, (usize, usize)>,
    /// Byte offset of the operator token for expression nodes that have one
    /// (e.g. `!` in postfix early-return, `+` in binary add).
    pub op_positions: HashMap<NodeId, usize>,
    /// Byte offsets of each line start (index 0 = line 1 at offset 0).
    /// Built during parsing; used to convert byte offsets to line numbers.
    pub line_offsets: Vec<usize>,
}

impl SourceMap {
    /// Convert a byte offset to a 1-based line number.
    /// Returns None if line_offsets is empty.
    pub fn byte_to_line(&self, byte_offset: usize) -> Option<usize> {
        if self.line_offsets.is_empty() {
            return None;
        }
        let line_idx = match self.line_offsets.binary_search(&byte_offset) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        Some(line_idx + 1)
    }
}

/// Spec 6.3 - import
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Import {
    pub node_id: NodeId,
    pub path: Vec<Ident>,
    pub alias: Option<Ident>,
}

/// Spec 6.4 - annotation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub node_id: NodeId,
    pub invocation: ExprInvocation,
}

/// Spec 6.1 - source_file
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceFile {
    pub node_id: NodeId,
    pub file_path: String,
    #[serde(skip)]
    pub source_map: SourceMap,
    pub comments: Vec<Comment>,
    pub annotations: Vec<Annotation>,
    pub imports: Vec<Import>,
    pub constructs: Vec<Construct>,
}

/// Spec 6.2 - module (logical module, may span multiple files)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Module {
    pub qualified_name: Vec<String>,
    pub files: Vec<SourceFile>,
}
