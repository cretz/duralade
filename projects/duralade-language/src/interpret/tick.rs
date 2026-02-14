use std::collections::BTreeMap;
use std::task::{RawWaker, RawWakerVTable, Waker};

use tracing::instrument;

use crate::engine::ModuleIndex;
use crate::event::{OwnedFields, OwnedValue, OwnedValueError};
use crate::load::TypeField;
use crate::model::Symbol;

use super::call;
use super::context::EvalContext;
use super::eval::{self, EvalResult};
use super::member;
use super::scope::{FaultError, ImplicitBinding, ImplicitInit, InternalError};
use super::value::ValueId;

pub(crate) fn convert_args(
    mut args: OwnedFields,
    decl_fields: &[TypeField],
    name: &str,
    ctx: &EvalContext,
) -> Result<BTreeMap<Symbol, super::value::Value>, OwnedValueError> {
    let mut result = BTreeMap::new();
    for field in decl_fields {
        if !matches!(
            field.modifier,
            Some(crate::load::FieldModifier::In) | Some(crate::load::FieldModifier::Inout)
        ) {
            continue;
        }
        let field_name = &field.name;
        if let Some(owned) = args.remove(field_name.as_str()) {
            let val = owned.into_value(field.ty, &ctx.registry, &mut ctx.heap_mut())?;
            result.insert(field_name.clone(), val);
        } else if crate::event::is_field_required(field, &ctx.registry) {
            return Err(OwnedValueError(format!(
                "missing required field '{}' for '{}'",
                field_name, name
            )));
        }
    }
    Ok(result)
}

#[derive(Debug)]
pub enum TickStatus {
    /// The entry function ran to completion. Result is in the EntityComplete event.
    Completed,
    /// Execution is blocked on an extern invocation.
    Blocked,
}

pub(crate) enum TickOutcome {
    Ok(OwnedValue),
    Fault(FaultError),
    InternalError(InternalError),
}

/// Convert an EvalResult to a TickOutcome, freeing any untracked heap
/// allocation in the result value after copying it to OwnedValue.
fn result_to_outcome(result: EvalResult, ctx: &mut EvalContext) -> TickOutcome {
    match result {
        EvalResult::Ok(v) => match OwnedValue::from_value(&v, &ctx.heap()) {
            Ok(owned) => TickOutcome::Ok(owned),
            Err(e) => TickOutcome::InternalError(InternalError::new(&e.0)),
        },
        EvalResult::Return => TickOutcome::InternalError(
            match eval::make_internal_error(
                "unexpected Return reached tick boundary".into(),
                vec![],
            ) {
                EvalResult::InternalError(e) => e,
                _ => unreachable!(),
            },
        ),
        EvalResult::Break(_) | EvalResult::Continue(_) => TickOutcome::InternalError(
            InternalError::new("unexpected break/continue reached tick boundary"),
        ),
        EvalResult::Fault(e) => TickOutcome::Fault(e),
        EvalResult::InternalError(e) => TickOutcome::InternalError(e),
    }
}

async fn eval_implicit_inits(
    ctx: &mut EvalContext,
    inits: Vec<ImplicitInit>,
) -> Result<Vec<ImplicitBinding>, TickOutcome> {
    let mut bindings = Vec::with_capacity(inits.len());
    for init in inits {
        let entry = ctx.registry.decl(init.decl);
        let module_idx = entry.module;
        let decl_name = entry.name.clone();
        ctx.push_import_context(module_idx);
        let result = call::call_or_construct(
            ctx,
            module_idx,
            &decl_name,
            super::scope::FuncInput::default(),
            None,
        )
        .await;
        ctx.pop_import_context();
        match result {
            EvalResult::Ok(value) => bindings.push(ImplicitBinding { ty: init.ty, value }),
            EvalResult::Fault(e) => return Err(TickOutcome::Fault(e)),
            EvalResult::InternalError(e) => return Err(TickOutcome::InternalError(e)),
            EvalResult::Return | EvalResult::Break(_) | EvalResult::Continue(_) => {
                return Err(TickOutcome::InternalError(InternalError::new(
                    "unexpected Return/break/continue in implicit init",
                )));
            }
        }
    }
    Ok(bindings)
}

#[instrument(level = "debug", skip_all, fields(func = %entry_func))]
pub(crate) async fn run_entry(
    mut ctx: EvalContext,
    module_idx: ModuleIndex,
    entry_func: Symbol,
    args: OwnedFields,
    implicit_inits: Vec<ImplicitInit>,
) -> TickOutcome {
    let implicits = match eval_implicit_inits(&mut ctx, implicit_inits).await {
        Ok(b) => b,
        Err(outcome) => return outcome,
    };
    let decl_fields = ctx
        .registry
        .module(module_idx)
        .get_declaration(&entry_func)
        .map(|d| d.fields.as_slice())
        .unwrap_or(&[]);
    let runtime_args = match convert_args(args, decl_fields, &entry_func, &ctx) {
        Ok(a) => a,
        Err(e) => return TickOutcome::InternalError(InternalError::new(&e.0)),
    };

    let input = super::scope::FuncInput {
        args: runtime_args,
        type_args: BTreeMap::new(),
        implicits,
    };

    ctx.push_import_context(module_idx);
    let result = call::call_or_construct(&mut ctx, module_idx, &entry_func, input, None).await;
    ctx.pop_import_context();

    result_to_outcome(result, &mut ctx)
}

#[instrument(level = "debug", skip_all, fields(entity = %entity_name))]
pub(crate) async fn run_entity_entry(
    mut ctx: EvalContext,
    module_idx: ModuleIndex,
    entity_name: Symbol,
    args: OwnedFields,
    entity_value_id: super::value::ValueId,
    implicit_inits: Vec<ImplicitInit>,
) -> TickOutcome {
    let entry_implicits = match eval_implicit_inits(&mut ctx, implicit_inits).await {
        Ok(b) => b,
        Err(outcome) => return outcome,
    };
    let decl_fields = ctx
        .registry
        .module(module_idx)
        .get_declaration(&entity_name)
        .map(|d| d.fields.as_slice())
        .unwrap_or(&[]);
    let runtime_args = match convert_args(args, decl_fields, &entity_name, &ctx) {
        Ok(a) => a,
        Err(e) => return TickOutcome::InternalError(InternalError::new(&e.0)),
    };

    let result = member::run_entity(
        &mut ctx,
        module_idx,
        &entity_name,
        runtime_args,
        entity_value_id,
        entry_implicits,
    )
    .await;

    result_to_outcome(result, &mut ctx)
}

#[instrument(level = "debug", skip_all, fields(entity = %entity_name, func = %func_name))]
pub(crate) async fn run_member_func_entry(
    mut ctx: EvalContext,
    module_idx: ModuleIndex,
    entity_name: Symbol,
    func_name: Symbol,
    args: OwnedFields,
    entity_value_id: ValueId,
    implicit_inits: Vec<ImplicitInit>,
) -> TickOutcome {
    let implicits = match eval_implicit_inits(&mut ctx, implicit_inits).await {
        Ok(b) => b,
        Err(outcome) => return outcome,
    };
    let decl_fields = ctx
        .registry
        .module(module_idx)
        .get_declaration(&entity_name)
        .and_then(|d| d.get_member(&func_name))
        .map(|m| m.fields.as_slice())
        .unwrap_or(&[]);
    let runtime_args = match convert_args(args, decl_fields, &func_name, &ctx) {
        Ok(a) => a,
        Err(e) => return TickOutcome::InternalError(InternalError::new(&e.0)),
    };

    let input = super::scope::FuncInput {
        args: runtime_args,
        type_args: BTreeMap::new(),
        implicits,
    };

    let result = member::run_entity_member_func(
        &mut ctx,
        module_idx,
        &entity_name,
        &func_name,
        input,
        BTreeMap::new(),
        entity_value_id,
    )
    .await;

    result_to_outcome(result, &mut ctx)
}

pub(crate) async fn run_override_func(
    mut ctx: EvalContext,
    func_id: super::value::ValueId,
    args: BTreeMap<Symbol, super::value::Value>,
) -> TickOutcome {
    let result = call::call_func_value(&mut ctx, func_id, args, None).await;
    result_to_outcome(result, &mut ctx)
}

pub(crate) fn noop_waker() -> Waker {
    fn noop(_: *const ()) {}
    fn clone(p: *const ()) -> RawWaker {
        RawWaker::new(p, &VTABLE)
    }
    static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, noop, noop, noop);
    unsafe { Waker::from_raw(RawWaker::new(core::ptr::null(), &VTABLE)) }
}
