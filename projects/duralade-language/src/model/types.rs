use serde::{Deserialize, Serialize};

use super::{Ident, InvalidNode, NodeId};

/// Spec 7.1 - type
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum Type {
    Named(TypeNamed),
    Nilable(TypeNilable),
    EntityRef(TypeEntityRef),
    Introspection(TypeIntrospection),
    Anon(TypeAnon),
    Invalid(InvalidNode),
}

impl Type {
    pub fn node_id(&self) -> NodeId {
        match self {
            Type::Named(t) => t.node_id,
            Type::Nilable(t) => t.node_id,
            Type::EntityRef(t) => t.node_id,
            Type::Introspection(t) => t.node_id,
            Type::Anon(t) => t.node_id,
            Type::Invalid(t) => t.node_id,
        }
    }
}

/// Spec 7.1 - type_named
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeNamed {
    pub node_id: NodeId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<Ident>,
    pub path: Vec<Ident>,
    pub type_args: Vec<TypeArgument>,
}

/// Spec 7.1 - type_argument
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeArgument {
    pub node_id: NodeId,
    pub name: Ident,
    pub value: Type,
}

/// Spec 7.2 - type_nilable
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeNilable {
    pub node_id: NodeId,
    pub inner: Box<Type>,
}

/// Spec 7.3 - type_entity_ref
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeEntityRef {
    pub node_id: NodeId,
    pub inner: Box<Type>,
}

/// Spec 7.4 - type_introspection
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeIntrospection {
    pub node_id: NodeId,
    pub inner: TypeNamed,
    pub op: TypeIntrospectionOp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeIntrospectionOp {
    Type,
    TypeIn,
    TypeOut,
}

/// Spec 7.5 - type_anonymous
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeAnon {
    pub node_id: NodeId,
    pub construct: TypeAnonConstruct,
    pub fields: Vec<TypeAnonField>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeAnonConstruct {
    Data,
    Func,
    FuncView,
    FuncNoblock,
    Entity,
}

/// Spec 7.5 - type_anonymous_field
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeAnonField {
    pub node_id: NodeId,
    pub modifier: Option<TypeAnonFieldModifier>,
    pub name: Option<Ident>,
    pub ty: Box<Type>,
    pub has_default: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeAnonFieldModifier {
    In,
    Inout,
    Out,
    OutEarly,
    Implicit,
}
