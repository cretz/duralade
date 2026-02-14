use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::event::OwnedFields;
use crate::model::{Construct, ConstructKind, Entity, EntityRun, Func, NodeId, SourceFile, Symbol};

use super::registry::{DeclId, TypeId};

/// A resolved annotation on a declaration or file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub decl: DeclId,
    pub fields: OwnedFields,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldModifier {
    In,
    Out,
    OutEarly,
    Inout,
    Value,
    Implicit,
}

impl FieldModifier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
            Self::OutEarly => "out!",
            Self::Inout => "inout",
            Self::Value => "value",
            Self::Implicit => "implicit",
        }
    }
}

/// A resolved field - used in both named constructs and anonymous types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeField {
    pub name: Symbol,
    pub modifier: Option<FieldModifier>,
    pub ty: TypeId,
    pub has_default: bool,
    pub node_id: NodeId,
}

/// A resolved type parameter field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TypeIntype {
    pub name: Symbol,
    pub constraint: Option<TypeId>,
    pub default: Option<TypeId>,
    pub node_id: NodeId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TypeConstructKind {
    TypeAlias,
    Data,
    Entity,
    Func,
    Extern,
    Native,
}

impl TypeConstructKind {
    pub fn is_func(&self) -> bool {
        matches!(self, Self::Func | Self::Extern | Self::Native)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Private,
}

/// A resolved declaration - a construct whose signature (field types) has
/// been fully resolved. Back-references the AST via NodeId.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TypeConstruct {
    #[serde(default)]
    pub name: Option<Symbol>,
    pub kind: TypeConstructKind,
    pub visibility: Visibility,
    pub annotations: Vec<Annotation>,
    pub intypes: Vec<TypeIntype>,
    pub fields: Vec<TypeField>,
    #[serde(default)]
    pub out_early_name: Option<Symbol>,
    #[serde(default)]
    pub out_field_names: Vec<Symbol>,
    pub func_modifiers: FuncModifiers,
    /// For type alias: the resolved target type.
    pub type_alias_target: Option<TypeId>,
    /// Nested declarations (member functions on data/entity).
    pub members: Vec<TypeConstruct>,
    #[serde(default)]
    pub member_index: HashMap<Symbol, usize>,
    /// True for `builtin` constructs (runtime-provided, not constructible).
    pub is_builtin: bool,
    /// True if this construct has non-serializable types (func/entity/anylocal)
    /// in its type graph. Computed after field type resolution.
    #[serde(default)]
    pub non_serializable: bool,
    /// The DeclId for this construct in the registry, set during loading.
    #[serde(default)]
    pub decl_id: Option<DeclId>,
    pub node_id: NodeId,
    /// Topologically sorted AST field indices for dependency-ordered evaluation.
    /// Indices are into the AST construct's `fields` array (not `self.fields`,
    /// which excludes intypes and invalids). Circular references are detected
    /// and reported as load errors at compile time.
    #[serde(default)]
    pub ast_field_eval_order: Vec<usize>,
}

impl TypeConstruct {
    pub fn rebuild_metadata(&mut self) {
        self.out_early_name = self
            .fields
            .iter()
            .find(|f| f.modifier == Some(FieldModifier::OutEarly))
            .map(|f| f.name.clone());
        self.out_field_names = self
            .fields
            .iter()
            .filter(|f| {
                matches!(
                    f.modifier,
                    Some(FieldModifier::Out)
                        | Some(FieldModifier::Inout)
                        | Some(FieldModifier::OutEarly)
                )
            })
            .map(|f| f.name.clone())
            .collect();
        self.member_index = self
            .members
            .iter()
            .enumerate()
            .filter_map(|(idx, member)| Some((member.name.clone()?, idx)))
            .collect();
    }

    pub fn get_member(&self, name: &str) -> Option<&TypeConstruct> {
        let &idx = self.member_index.get(name)?;
        self.members.get(idx)
    }

    pub fn get_member_mut(&mut self, name: &str) -> Option<&mut TypeConstruct> {
        let &idx = self.member_index.get(name)?;
        self.members.get_mut(idx)
    }

    pub fn get_implicit_field_type(&self, field_name: &str) -> Option<TypeId> {
        self.fields
            .iter()
            .find(|f| f.name == field_name && f.modifier == Some(FieldModifier::Implicit))
            .map(|f| f.ty)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct FuncModifiers {
    pub view: bool,
    pub noblock: bool,
}

/// A source file with its resolved type information.
#[derive(Debug, Serialize, Deserialize)]
pub struct TypedFile {
    pub source_file: SourceFile,
    pub type_table: TypeTable,
    pub annotations: Vec<Annotation>,
    pub declarations: Vec<TypeConstruct>,
    /// Resolved import alias → module path for this file. Includes both
    /// explicit imports from source and implicit imports (stdlib prelude).
    /// Explicit imports shadow implicit ones with the same alias.
    pub import_map: HashMap<Symbol, Symbol>,
}

/// A loaded module, potentially type-checked.
#[derive(Debug, Serialize, Deserialize)]
pub struct Module {
    /// True if the module had errors during loading (fetch, parse, etc.).
    /// When true, declarations may be incomplete - downstream "not found"
    /// errors should mention this rather than claiming the name doesn't exist.
    pub has_errors: bool,
    pub files: Vec<TypedFile>,
    /// Maps declaration name → (file_index, decl_index) into files.
    pub decl_index: HashMap<Symbol, (usize, usize)>,
}

impl Module {
    fn get_ast_construct(&self, name: &str) -> Option<&Construct> {
        let &(fi, di) = self.decl_index.get(name)?;
        self.files[fi]
            .source_file
            .constructs
            .iter()
            .filter(|c| !matches!(c.kind, ConstructKind::Invalid(_)))
            .nth(di)
    }

    pub fn get_declaration(&self, name: &str) -> Option<&TypeConstruct> {
        let &(fi, di) = self.decl_index.get(name)?;
        self.files.get(fi)?.declarations.get(di)
    }

    /// Like `get_declaration` but also returns the file index (for StackFrame).
    pub fn get_declaration_indexed(&self, name: &str) -> Option<(u16, &TypeConstruct)> {
        let &(fi, di) = self.decl_index.get(name)?;
        let decl = self.files.get(fi)?.declarations.get(di)?;
        Some((fi as u16, decl))
    }

    pub fn get_declaration_mut(&mut self, name: &str) -> Option<&mut TypeConstruct> {
        let &(fi, di) = self.decl_index.get(name)?;
        self.files.get_mut(fi)?.declarations.get_mut(di)
    }

    pub fn get_func_ast(&self, name: &str) -> Option<&Func> {
        match &self.get_ast_construct(name)?.kind {
            ConstructKind::Func(func) => Some(func),
            _ => None,
        }
    }

    pub fn get_entity_ast(&self, name: &str) -> Option<&Entity> {
        match &self.get_ast_construct(name)?.kind {
            ConstructKind::Entity(entity) => Some(entity),
            _ => None,
        }
    }

    pub fn get_data_member_ast(&self, data_name: &str, member_name: &str) -> Option<&Construct> {
        self.files
            .iter()
            .flat_map(|f| &f.source_file.constructs)
            .filter_map(|c| match &c.kind {
                ConstructKind::Data(data) if c.name.name == data_name => {
                    data.funcs.iter().find(|f| f.name.name == member_name)
                }
                _ => None,
            })
            .next()
    }

    pub fn get_entity_member_ast(
        &self,
        entity_name: &str,
        member_name: &str,
    ) -> Option<&Construct> {
        self.get_entity_ast(entity_name)?
            .funcs
            .iter()
            .find(|f| f.name.name == member_name)
    }

    pub fn get_entity_run_ast(&self, entity_name: &str) -> Option<&EntityRun> {
        self.get_entity_ast(entity_name)?.run.as_ref()
    }
}

/// Side table mapping AST nodes to resolved type information.
/// The AST itself is never modified - all type info lives here.
///
/// `implicitly` statement expression types, anon func/data expression
/// types, and narrowing target types (expression `as` and if-narrowing
/// bindings) are always populated (the interpreter needs them at runtime).
/// All other expression types are populated only when
/// `populate_non_required_types` is set (e.g., for LSP hover support).
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TypeTable {
    pub node_types: HashMap<NodeId, TypeId>,
}
