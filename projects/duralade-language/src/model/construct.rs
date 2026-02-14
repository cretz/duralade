use serde::{Deserialize, Serialize};

use super::{Annotation, Expr, Ident, InvalidNode, NodeId, Stmt, Type};

/// Spec 8.1 - file_construct
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Construct {
    pub node_id: NodeId,
    pub annotations: Vec<Annotation>,
    pub out: bool,
    pub builtin: bool,
    pub name: Ident,
    pub kind: ConstructKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ConstructKind {
    TypeAlias(TypeAlias),
    Data(Data),
    Entity(Entity),
    Func(Func),
    Extern(Extern),
    Native(Native),
    Invalid(InvalidNode),
}

/// Spec 8.2 - field
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Field {
    pub node_id: NodeId,
    pub annotations: Vec<Annotation>,
    pub kind: FieldKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum FieldKind {
    Intype(FieldIntype),
    Var(FieldVar),
    Invalid(InvalidNode),
}

/// Spec 8.2 - field_intype
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldIntype {
    pub node_id: NodeId,
    pub name: Ident,
    pub constraint: Option<Type>,
    pub default: Option<Type>,
}

/// Spec 8.2 - field_var
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldVar {
    pub node_id: NodeId,
    pub modifier: FieldVarModifier,
    pub name: Ident,
    pub ty: Option<Type>,
    pub default: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldVarModifier {
    In,
    Inout,
    Out,
    OutEarly,
    Implicit,
    Value,
}

/// Spec 8.3 - type_alias
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeAlias {
    pub node_id: NodeId,
    pub fields: Vec<FieldIntype>,
    pub target: Type,
}

/// Spec 8.4 - data
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Data {
    pub node_id: NodeId,
    pub fields: Vec<Field>,
    pub funcs: Vec<Construct>,
}

/// Spec 8.5 - entity
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entity {
    pub node_id: NodeId,
    pub fields: Vec<Field>,
    pub init: Option<EntityInit>,
    pub run: Option<EntityRun>,
    pub funcs: Vec<Construct>,
}

/// Spec 8.5 - entity_init
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityInit {
    pub node_id: NodeId,
    pub fields: Vec<Field>,
    pub stmts: Vec<Stmt>,
}

/// Spec 8.5 - entity_run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityRun {
    pub node_id: NodeId,
    pub fields: Vec<Field>,
    pub stmts: Vec<Stmt>,
}

/// Spec 8.6 - func
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Func {
    pub node_id: NodeId,
    pub view: bool,
    pub noblock: bool,
    pub fields: Vec<Field>,
    pub stmts: Vec<Stmt>,
}

/// Spec 8.7 - extern
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Extern {
    pub node_id: NodeId,
    pub fields: Vec<Field>,
}

/// Spec 8.7 - native
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Native {
    pub node_id: NodeId,
    pub view: bool,
    pub noblock: bool,
    pub fields: Vec<Field>,
}
