use duralade_runtime::local_engine::{LocalEngine, LocalEngineOptions};
use duralade_runtime::project::IMPLICIT_ROOT;

use crate::project;
use crate::state::{self, Store};

pub struct EngineOptions<'a> {
    pub code: Option<&'a str>,
    pub state: &'a str,
    pub state_out: Option<&'a str>,
    pub dry_run: bool,
    pub strict: bool,
    pub disable_heap_collect: bool,
}

// TODO: support remote/hosted engine - EngineHandle will need to become
// trait-object-based or an enum once we have more than LocalEngine.
pub struct EngineHandle {
    pub engine: LocalEngine<Store>,
    is_implicit: bool,
}

impl EngineHandle {
    // TODO: engine selection (local vs remote) will be determined here,
    // e.g. by state URI scheme or an explicit --engine flag.
    pub fn new(opts: EngineOptions) -> Result<Self, String> {
        let in_url = state::parse_state_uri(opts.state)?;
        let out_url = opts.state_out.map(state::parse_state_uri).transpose()?;
        let store = Store::open(&in_url, out_url.as_ref(), opts.dry_run)?;

        let (registry, is_implicit) = match opts.code {
            Some(c) => {
                let loaded = project::load_code(c, opts.strict)?;
                (loaded.registry, loaded.is_implicit)
            }
            None => (std::sync::Arc::default(), false),
        };

        Ok(Self {
            engine: LocalEngine::new(
                registry,
                store,
                LocalEngineOptions {
                    disable_heap_collect: opts.disable_heap_collect,
                    cache_capacity: 0,
                    cache_completed: false,
                },
            ),
            is_implicit,
        })
    }

    pub fn qualify_entity(&self, name: &str) -> String {
        if self.is_implicit {
            format!("{IMPLICIT_ROOT}.{name}")
        } else {
            name.to_string()
        }
    }
}
