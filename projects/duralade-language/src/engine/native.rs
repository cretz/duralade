use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use crate::interpret::context::EvalContext;
use crate::interpret::eval::EvalResult;
use crate::interpret::scope::{FaultError, InternalError};
use crate::interpret::value::Value;
use crate::model::Symbol;

pub type NativeArgs = BTreeMap<Symbol, Value>;
pub type NativeOut = BTreeMap<Symbol, Value>;

/// Native handler failure integrates with interpreter error categories.
pub enum NativeCallError {
    Fault(FaultError),
    InternalError(InternalError),
    /// Control flow (break/continue/return) that must propagate through native calls.
    ControlFlow(EvalResult),
}

pub type NativeCallResult = Result<(), NativeCallError>;
pub type NativeCallFuture<'a> = Pin<Box<dyn Future<Output = NativeCallResult> + 'a>>;

pub struct NativeCallContext<'a> {
    pub eval_ctx: &'a mut EvalContext,

    // The "self" value when this native is a member function on a data/primitive type.
    // Only set by the runtime for stdlib built-in member functions (e.g. str.index_of).
    // None for all other native calls.
    construct_value: Option<Value>,
}

impl<'a> NativeCallContext<'a> {
    pub fn new(eval_ctx: &'a mut EvalContext) -> Self {
        Self {
            eval_ctx,
            construct_value: None,
        }
    }

    pub fn with_construct_value(eval_ctx: &'a mut EvalContext, value: Value) -> Self {
        Self {
            eval_ctx,
            construct_value: Some(value),
        }
    }

    /// The "self" value for data/primitive member native calls (stdlib built-ins).
    /// Returns None for all non-member native calls.
    pub fn construct_value(&self) -> Option<&Value> {
        self.construct_value.as_ref()
    }
}

pub trait NativeHandler: 'static {
    fn call<'a>(
        &'a self,
        ctx: &'a mut NativeCallContext<'a>,
        args: &'a NativeArgs,
        out: &'a mut NativeOut,
    ) -> NativeCallFuture<'a>;
}

impl<F> NativeHandler for F
where
    F: for<'a> Fn(
            &'a mut NativeCallContext<'a>,
            &'a NativeArgs,
            &'a mut NativeOut,
        ) -> NativeCallFuture<'a>
        + 'static,
{
    fn call<'a>(
        &'a self,
        ctx: &'a mut NativeCallContext<'a>,
        args: &'a NativeArgs,
        out: &'a mut NativeOut,
    ) -> NativeCallFuture<'a> {
        (self)(ctx, args, out)
    }
}

/// Registry keyed by native qualified name, e.g. `duralade.test::on_extern_native`.
#[derive(Default)]
pub struct NativeRegistry {
    handlers: HashMap<String, Arc<dyn NativeHandler>>,
}

impl std::fmt::Debug for NativeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeRegistry")
            .field("handler_count", &self.handlers.len())
            .finish()
    }
}

impl NativeRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(
        &mut self,
        qualified_name: impl Into<String>,
        handler: impl NativeHandler,
    ) -> Result<(), String> {
        let name = qualified_name.into();
        if self.handlers.contains_key(&name) {
            return Err(format!("native handler already registered: {name}"));
        }
        self.handlers.insert(name, Arc::new(handler));
        Ok(())
    }

    pub fn has(&self, qualified_name: &str) -> bool {
        self.handlers.contains_key(qualified_name)
    }

    pub fn get(&self, qualified_name: &str) -> Option<Arc<dyn NativeHandler>> {
        self.handlers.get(qualified_name).cloned()
    }
}
