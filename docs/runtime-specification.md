
Notes:

* Fobs/funcs keep track not only of state, but where in stack and children/references to started fobs
* Full step debuggability, replayability, and runtime introspection
* Can do a "tick" which is "run until all yielded"
  * TODO(cretz): Support a "step" for single step? Maybe only when debugging, not for side effecting things?
* Entire program always serializable/resumable
* Explain `cancel.token` and `implicit`s`/implicitly`
* TODO: Early layout of std lib
    * Prelude?
    * `error`
    * `cancel` (and `cancel.token`)
    * `sync` including common async constructs (any, all, lock)
* Explain how basically everyone uses `out! :error?`