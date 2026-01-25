use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::{Comment, Construct, ExprInvocation, Ident, NodeId};

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceFile {
    pub node_id: NodeId,
    pub file_path: String,
    pub source_map: HashMap<NodeId, (usize, usize)>,
    pub comments: Vec<Comment>,
    pub annotations: Vec<Annotation>,
    pub imports: Vec<Import>,
    pub constructs: Vec<Construct>,
}

/// Spec 6.2 - module (logical module, may span multiple files)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Module {
    pub qualified_name: Vec<String>,
    pub files: Vec<SourceFile>,
}
