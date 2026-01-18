# Duralade Runtime Notes

This document contains design notes, open questions, and TODOs for the Duralade runtime. For the actual runtime
specification, see `runtime-spec.md` (currently TODO).

## Table of Contents

1. [Runtime Overview](#1-runtime-overview)
2. [Module System](#2-module-system)
3. [Type System](#3-type-system)
4. [Entities and Entity References](#4-entities-and-entity-references)
5. [Externs and Natives](#5-externs-and-natives)
6. [Standard Library and Builtins](#6-standard-library-and-builtins)
7. [AI Code Generation Patterns](#7-ai-code-generation-patterns)

## 1. Runtime Overview

TODO: Describe the Duralade runtime environment, execution model, and general architecture.

Topics to cover:

- CLI interface and commands (compile, run, step, tick, debug, etc.)
- Rust library/implementation architecture
- How the runtime is structured (compiler, interpreter/VM, execution engine)
- Relationship between CLI tool and library APIs

## 2. Module System

TODO: Describe how modules are organized on disk, resolved, and loaded.

Topics to cover:

- Project structure and `duralade.toml` (or equivalent)
- Directory structure and module paths
- Module resolution rules
- Dependency management:
  - Dependencies specified in `duralade.toml` with URL and version
  - Convention: dependency name should be last URL segment converted to snake_case
  - Dependencies can point to subdirectories within a repository
  - How subdirectory paths affect suggested module names
  - Versioning assumptions (commit hashes and version tags are immutable once published)
  - No centralized package manager - direct repository references
  - Tooling for checking naming conventions

## 3. Type System

TODO: Describe runtime behavior of types.

Topics to cover:

- Built-in type semantics (`int` - unbounded signed integer, `float` - 64-bit IEEE 754 without NaN/Infinity, `str`,
  `bool`, `any`)
- Type values and the builtin `type` type (first-class types)
- Type serialization: types serialize as qualified names (e.g., `"my_module.user"`)
- Type operators: `@type`, `@typein`, `@typeout` (TBD)
- Serialization format (JSON by default, pluggable)
- Type operations and methods
- Memory model and garbage collection

## 4. Entities and Entity References

TODO: Describe entity lifecycle, execution model, and entity references.

Topics to cover:

### Entity Construction and Execution

- Local entity construction vs spawned entities
- Entity construction is implicitly `noblock` - `init` runs inline, `run` starts as background coroutine
- When passed as a parameter, only `out`/`inout` fields and `view` functions are accessible
- Entities with `run` functions complete when `run` returns; entities without `run` run indefinitely

### Entity References (`entity_ref`)

- `entity_ref` is a reference to a spawned, detached entity (local or remote)
- Returned by the `spawn` builtin
- Entity refs provide:
  - `.id` - The entity's identifier
  - `->` operator for calling member functions on the referenced entity
  - `.cancel()` - Request cancellation of the entity
  - `.terminate()` - Force termination of the entity
  - Access to `out`/`inout` fields
- Local entities can coerce to `entity_ref` type when needed
- Entity refs cannot be used in `view` or `noblock` functions (unlike local entities without `run`)

### Spawn Builtin Function

- `spawn` is a builtin function that creates detached, identifiable entities
- Accepts:
  - An entity specification expression (first parameter)
  - Additional named parameters for spawn configuration (e.g., `id`, `target`)
- Returns an `entity_ref` to the spawned entity

### Entity Specification Expressions

- Entity specification semantics TBD. Current thinking: entity construction expressions like `my_entity(x: 1, y: 2)` are
  context-dependent. When used as a normal expression, they construct and return a local entity. When passed to `spawn`
  (which expects an `entity_spec[T]` type), the same syntax represents a specification without immediate execution.
- This is similar to how C# handles lambda expressions vs expression trees based on the expected delegate/expression
  type.
- The type system needs to support entity spec types to enable this behavior.

### TODO: Unresolved Design Questions

**Entity Types:**

- Should `entity` and `entity_ref` be the actual type names, or should they be anonymous/structural types (like
  TypeScript interfaces or Go interfaces)?
- How do anonymous entity ref types work for type hinting?
- Syntax for entity reference types (e.g., `entity_ref[my_entity]`)?
- Types representing inputs and outputs of entities and functions (e.g., `my_entity:in`, `my_func:out`) - syntax and
  semantics?
- Current thinking: Entity refs should be language-level types with `->` access syntax (not stdlib) to make
  blocking/remote access explicit. `local_entity.method()` is non-blocking, but `entity_ref->method()` is blocking,
  making the distinction clear. `spawn` (stdlib) would return language-level `entity_ref[T]` types.

**Entity Specification:**

- What is the syntax for entity spec types (e.g., `entity_spec[my_entity]`, `spec[my_entity]`, or something else)?
- Can entity specs be stored in variables, passed to other functions, or are they only valid as immediate spawn
  arguments?
- Should there be a generic `info[some_entity]` type that represents an entity's parameters?
- Should entity creation expressions be a specific type or context-dependent (like C# expression trees)?
- Should entity specs require explicit syntax (e.g., `&my_entity(x: 1)` with `&` prefix) or be implicit based on
  parameter type (e.g., `spawn(my_entity(x: 1))` where spawn expects entity spec)?
- Trade-offs: Explicit syntax is clearer but more verbose. Implicit (context-dependent) is more flexible but harder to
  model and potentially confusing.

**Spawn Parameters:**

- What spawn parameters should exist (e.g., `id`, `target`)?
- Should these be language-level or stdlib conventions?
- What is `target`? A string identifier for an endpoint/location that the runtime maps to actual execution location?
- What happens when you spawn an entity without specifying an `id`? Is one auto-generated?
- Can you spawn the same `id` twice? What happens?

**Entity Lifecycle:**

- What are the exact semantics of `.cancel()` vs `.terminate()`?
- Garbage collection behavior for spawned entities - when are they cleaned up?
- How do you wait for or observe completion of a spawned entity?
- Can spawned entities be serialized/resumed across runtime restarts?
- How do you get a reference to an existing entity by ID (e.g., `get_entity(id: "my-id")` or similar)?
- What other remote entity operations are needed besides `spawn` (e.g., querying running entities, listing by status)?

## 5. Externs and Natives

TODO: Describe how externs and natives are implemented and how they interface with the runtime.

Topics to cover:

### Extern Implementation

- Externs are blocking, side-effecting, non-deterministic operations
- Runtime memoization: how are extern results stored and retrieved for deterministic replay?
- Memoization key: what identifies a unique extern call (function name, parameters, execution context)?
- How does the runtime handle extern failures during replay vs initial execution?
- What serialization format is used for extern inputs and outputs?
- Can externs be retried? What retry semantics exist?
- How are externs executed (in-process, separate process, remote service)?

### Native Implementation

- Natives are trusted deterministic implementations
- How are native functions registered with the runtime?
- What implementation mechanisms are supported:
  - Rust FFI (most likely, since runtime will be written in Rust)
  - Dynamic libraries (.dll, .so)?
  - Statically linked?
  - Runtime plugin system?
- What constraints exist on native function implementations to ensure determinism?
- How does the runtime verify or enforce `view` vs `noblock` constraints on natives?
- Are natives allowed to allocate memory, access thread-local storage, etc.?
- How are native panics/errors handled?

### Common Concerns

- Type marshalling between Duralade types and native implementations
- How are complex types (data, entities) passed to externs/natives?
- Performance considerations for calling externs vs natives
- Debugging and observability for extern/native calls
- Version compatibility: what happens when extern/native signatures change?

## 6. Standard Library and Builtins

TODO: Describe standard library types, functions, and builtin operations.

Topics to cover:

### Tasks and Coroutines

- `task[out: t]` type - Represents a local coroutine handle (contrast with entity refs which are remote)
- `start` builtin function - Starts a function as a background coroutine, returns `task`
  - Example: `start(background_work(item: 5))` returns `task[out: background_work@out]`
  - Can be called from `noblock` functions (starting doesn't block)
  - Tasks are local coroutines within the same process/runtime (not remote like entity refs)
  - Use `.` operator for field access (not `->` which is for remote entity refs)
  - Blocking via `wait` or accessing task fields
- Task operations and helpers (wait, combinators, etc.)

### Spawn

- `spawn` (or `spawn_entity`) builtin function - Creates durable, identifiable, potentially remote entities
- Returns entity reference `&entity_type`
- Accepts entity construction expression and optional parameters (`id`, `target`, etc.)
- Contrast with `start` which is lightweight and local

### Other Builtins

- Iterator protocol and `yielder[t]` type
- Collection types (arrays, maps) if builtin
- Standard error types
- Other core functionality

## 7. AI Code Generation Patterns

TODO: Define best practices and patterns to make Duralade code AI-code-generation-friendly.

While these patterns benefit all developers, they're especially important for AI code generation where the language
provides freedom of choice. The goal is to establish clear, consistent conventions.

Topics to cover:

### Error Handling Patterns

- Standard use of `out! :error?` for error returns
- When to use early returns vs regular returns
- How to structure error types (should there be a standard error data type?)
- Patterns for error propagation through call chains

### Cancellation Patterns

- Standard use of implicit cancellation tokens
- How cancellation should flow through entity hierarchies
- Patterns for respecting cancellation in long-running operations
- Whether there should be a standard `cancellation_token` type

### Entity Usage Patterns

- When to use entities vs functions
- Encouragement to use entities for operations that benefit from durability
- Patterns for structuring entity `in`/`out`/`value` fields
- Common patterns for `init` and `run` functions
- **Data vs Entity decision guidance:**
  - Use `data` for immutable or simple structures with no internal state management
  - Use `entity` when you need:
    - Mutable internal state (`value` fields)
    - Initialization logic (`init` function)
    - Background execution (`run` function)
    - Long-lived or durable state
  - Examples: semaphore, set, queue = entities; user record, configuration = data

### Code Organization

- Module structure best practices
- When to split functionality into multiple entities vs single entity with methods
- Naming conventions beyond what the language enforces
- Patterns for composing functionality
- **Naming conventions (best practice):**
  - Data types: `<noun>_<adjective>` pattern (e.g., `error_stale_prepared_values`, `shopping_cart_item`)
  - Functions: `<noun>_<verb>` pattern (e.g., `item_add`, `items_get`, `checkout_prepare`)
  - Externs: `<noun>_<verb>` pattern (e.g., `product_lookup`, `checkout_apply`)

### Other Conventions

- Common patterns for using `implicitly` (logging, tracing, auth context)
- Standard approaches for configuration and dependency injection
- Patterns for testing entities and functions
- Documentation conventions (since comments serve as documentation)
