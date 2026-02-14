use serde::{Deserialize, Serialize};

use super::{Data, Func, Ident, InvalidNode, NodeId, Type, TypeArgument};

/// Spec 10 - expression
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Expr {
    Paren(ExprParen),
    Access(ExprAccess),
    ModuleAccess(ExprModuleAccess),
    Ident(Ident),
    Outer(ExprOuter),
    Unary(ExprUnary),
    Binary(ExprBinary),
    Narrowing(ExprNarrowing),
    Invocation(ExprInvocation),
    Literal(ExprLiteral),
    Wait(ExprWait),
    Spawn(ExprSpawn),
    AnonData(Data),
    AnonFunc(Func),
    Invalid(InvalidNode),
}

impl Expr {
    pub fn node_id(&self) -> NodeId {
        match self {
            Expr::Paren(e) => e.node_id,
            Expr::Access(e) => e.node_id,
            Expr::ModuleAccess(e) => e.node_id,
            Expr::Ident(e) => e.node_id,
            Expr::Outer(e) => e.node_id,
            Expr::Unary(e) => e.node_id,
            Expr::Binary(e) => e.node_id,
            Expr::Narrowing(e) => e.node_id,
            Expr::Invocation(e) => e.node_id,
            Expr::Literal(e) => e.node_id,
            Expr::Wait(e) => e.node_id,
            Expr::Spawn(e) => e.node_id,
            Expr::AnonData(e) => e.node_id,
            Expr::AnonFunc(e) => e.node_id,
            Expr::Invalid(e) => e.node_id,
        }
    }
}

/// Spec 10.1 - expression_parenthesized
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprParen {
    pub node_id: NodeId,
    pub expr: Box<Expr>,
}

/// Spec 10.1 - expression_access
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprAccess {
    pub node_id: NodeId,
    pub expr: Box<Expr>,
    pub op: ExprAccessOp,
    pub ident: Ident,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExprAccessOp {
    Dot,
    NilSafe,
    EntityRef,
}

/// Spec 6.2 - module_access: `module::name`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprModuleAccess {
    pub node_id: NodeId,
    pub module: Ident,
    pub name: Ident,
}

/// Spec 10.2 - outer
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprOuter {
    pub node_id: NodeId,
    pub depth: usize,
}

/// Spec 10.3 - unary
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprUnary {
    pub node_id: NodeId,
    pub op: ExprUnaryOp,
    pub expr: Box<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExprUnaryOp {
    Negate,
    Not,
    EarlyReturn,
}

/// Spec 10.4 - binary
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprBinary {
    pub node_id: NodeId,
    pub left: Box<Expr>,
    pub op: ExprBinaryOp,
    pub right: Box<Expr>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ExprBinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    NilCoalesce,
}

impl ExprBinaryOp {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::And => "&&",
            Self::Or => "||",
            Self::NilCoalesce => "??",
        }
    }
}

/// Spec 10.5 - narrowing
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprNarrowing {
    pub node_id: NodeId,
    pub expr: Box<Expr>,
    pub kind: ExprNarrowingKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ExprNarrowingKind {
    Nil(ExprNarrowingNil),
    As(ExprNarrowingAs),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprNarrowingNil {
    pub node_id: NodeId,
    pub else_expr: Option<Box<Expr>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprNarrowingAs {
    pub node_id: NodeId,
    pub ty: Type,
    pub else_expr: Option<Box<Expr>>,
}

/// Spec 10.6 - invocation
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprInvocation {
    pub node_id: NodeId,
    pub expr: Box<Expr>,
    pub type_args: Vec<TypeArgument>,
    pub args: Vec<ExprInvocationArg>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprInvocationArg {
    pub node_id: NodeId,
    pub name: Ident,
    pub value: Expr,
}

/// Spec 10.7 - literal
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprLiteral {
    pub node_id: NodeId,
    pub kind: ExprLiteralKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ExprLiteralKind {
    Int(String),
    Float(String),
    Str(String),
    Bool(bool),
    Nil,
    Array(Vec<Expr>),
    Map(Vec<ExprLiteralMapEntry>),
}

impl ExprLiteralKind {
    pub fn parse_int(s: &str) -> Result<i64, String> {
        s.parse::<i64>().or_else(|_| {
            if s.contains('_') {
                let cleaned: String = s.chars().filter(|&c| c != '_').collect();
                cleaned.parse::<i64>()
            } else {
                Err(s.parse::<i64>().unwrap_err())
            }
            .map_err(|_| format!("invalid integer: {s}"))
        })
    }

    pub fn parse_float(s: &str) -> Result<f64, String> {
        s.parse::<f64>().or_else(|_| {
            if s.contains('_') {
                let cleaned: String = s.chars().filter(|&c| c != '_').collect();
                cleaned.parse::<f64>()
            } else {
                Err(s.parse::<f64>().unwrap_err())
            }
            .map_err(|_| format!("invalid float: {s}"))
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprLiteralMapEntry {
    pub node_id: NodeId,
    pub key: Expr,
    pub value: Expr,
}

/// Spec 10.8 - wait
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprWait {
    pub node_id: NodeId,
    pub condition: Box<Expr>,
}

/// Spec 10.9 - spawn
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExprSpawn {
    pub node_id: NodeId,
    pub invocation: Box<Expr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<Box<Expr>>,
}
