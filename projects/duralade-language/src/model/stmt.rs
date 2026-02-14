use serde::{Deserialize, Serialize};

use super::{Expr, Ident, InvalidNode, NodeId, Type};

/// Spec 9 - statement
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Stmt {
    Block(StmtBlock),
    Var(StmtVar),
    Assign(StmtAssign),
    If(StmtIf),
    For(StmtFor),
    ForBreak(StmtForBreak),
    ForContinue(StmtForContinue),
    Defer(StmtDefer),
    Return(StmtReturn),
    ReturnEarly(StmtReturnEarly),
    Implicitly(StmtImplicitly),
    Patch(StmtPatch),
    Expr(Expr),
    Invalid(InvalidNode),
}

impl Stmt {
    pub fn node_id(&self) -> NodeId {
        match self {
            Stmt::Block(s) => s.node_id,
            Stmt::Var(s) => s.node_id,
            Stmt::Assign(s) => s.node_id,
            Stmt::If(s) => s.node_id,
            Stmt::For(s) => s.node_id,
            Stmt::ForBreak(s) => s.node_id,
            Stmt::ForContinue(s) => s.node_id,
            Stmt::Defer(s) => s.node_id,
            Stmt::Return(s) => s.node_id,
            Stmt::ReturnEarly(s) => s.node_id,
            Stmt::Implicitly(s) => s.node_id,
            Stmt::Patch(s) => s.node_id,
            Stmt::Expr(e) => e.node_id(),
            Stmt::Invalid(e) => e.node_id,
        }
    }
}

/// Spec 9.2 - block
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtBlock {
    pub node_id: NodeId,
    pub stmts: Vec<Stmt>,
}

/// Spec 9.3 - var
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtVar {
    pub node_id: NodeId,
    pub vars: Vec<StmtVarDecl>,
    pub values: Vec<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtVarDecl {
    pub node_id: NodeId,
    pub name: Ident,
    pub ty: Option<Type>,
}

/// Spec 9.3 - assignment
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtAssign {
    pub node_id: NodeId,
    pub vars: Vec<Expr>,
    pub op: StmtAssignOp,
    pub values: Vec<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StmtAssignOp {
    Assign,
    AddAssign,
    SubAssign,
    MulAssign,
    DivAssign,
    ModAssign,
}

/// Spec 9.4 - if
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtIf {
    pub node_id: NodeId,
    pub condition: StmtIfCondition,
    pub then_block: StmtBlock,
    pub else_ifs: Vec<StmtIfElseIf>,
    pub else_block: Option<StmtBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtIfElseIf {
    pub node_id: NodeId,
    pub condition: StmtIfCondition,
    pub then_block: StmtBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StmtIfCondition {
    Bool(StmtIfConditionBool),
    NarrowingAs(StmtIfNarrowingAs),
    NarrowingNil(StmtIfNarrowingNil),
}

/// Spec 9.4 - if_condition_bool
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtIfConditionBool {
    pub node_id: NodeId,
    pub init: Option<Box<Stmt>>,
    pub expr: Expr,
}

/// Spec 9.4 - if_narrowing_as
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtIfNarrowingAs {
    pub node_id: NodeId,
    pub bindings: Vec<StmtIfNarrowingAsBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtIfNarrowingAsBinding {
    pub node_id: NodeId,
    pub name: Ident,
    pub expr: Expr,
    pub ty: Type,
}

/// Spec 9.4 - if_narrowing_nil
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtIfNarrowingNil {
    pub node_id: NodeId,
    pub bindings: Vec<StmtIfNarrowingNilBinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtIfNarrowingNilBinding {
    pub node_id: NodeId,
    pub name: Ident,
    pub expr: Expr,
}

/// Spec 9.5 - for
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtFor {
    pub node_id: NodeId,
    pub label: Option<Ident>,
    pub clause: Option<StmtForClause>,
    pub block: StmtBlock,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StmtForClause {
    Condition(StmtForCondition),
    In(StmtForIn),
}

/// Spec 9.5 - for_clause (condition form)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtForCondition {
    pub node_id: NodeId,
    pub expr: Expr,
}

/// Spec 9.5 - for_clause (in form). Expression evaluates to an iter func.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtForIn {
    pub node_id: NodeId,
    pub var: Ident,
    pub expr: Expr,
}

/// Spec 9.5 - for_break
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtForBreak {
    pub node_id: NodeId,
    pub label: Option<Ident>,
}

/// Spec 9.5 - for_continue
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtForContinue {
    pub node_id: NodeId,
    pub label: Option<Ident>,
}

/// Spec 9.6 - defer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtDefer {
    pub node_id: NodeId,
    pub block: StmtBlock,
}

/// Spec 9.7 - return
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtReturn {
    pub node_id: NodeId,
}

/// Spec 9.7 - return_early
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtReturnEarly {
    pub node_id: NodeId,
    pub expr: Expr,
}

/// Spec 9.8 - implicitly
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtImplicitly {
    pub node_id: NodeId,
    pub exprs: Vec<Expr>,
    pub block: Option<StmtBlock>,
}

/// Spec 9.9 - patch
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtPatch {
    pub node_id: NodeId,
    pub kind: StmtPatchKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum StmtPatchKind {
    Chain(StmtPatchChain),
    Complete(StmtPatchComplete),
}

/// Spec 9.9 - patch_chain
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtPatchChain {
    pub node_id: NodeId,
    pub branches: Vec<StmtPatchBranch>,
}

/// Spec 9.9 - patch_branch
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtPatchBranch {
    pub node_id: NodeId,
    pub version: Vec<Ident>,
    pub stmts: Vec<Stmt>,
}

/// Spec 9.9 - patch_complete
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StmtPatchComplete {
    pub node_id: NodeId,
    pub version: Vec<Ident>,
}
