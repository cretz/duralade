# Duralade Engine Specification

## Table of Contents

1. [Overview](#1-overview)
2. [Project Structure](#2-project-structure)
   - 2.1. [Overview](#21-overview)
   - 2.2. [Project Configuration](#22-project-configuration)
   - 2.3. [Directory Layout](#23-directory-layout)
   - 2.4. [Visibility and Access Control](#24-visibility-and-access-control)
   - 2.5. [Dependencies](#25-dependencies)
3. [Build System](#3-build-system)
   - 3.1. [Bundle Format](#31-bundle-format)
   - 3.2. [Shared Libraries](#32-shared-libraries)
4. [Execution Model](#4-execution-model)
   - 4.1. [Overview](#41-overview)
   - 4.2. [Entity Lifecycle](#42-entity-lifecycle)
   - 4.3. [Core Operations](#43-core-operations)
     - 4.3.1. [Overview](#431-overview)
     - 4.3.2. [`duralade entity spawn`](#432-duralade-entity-spawn)
     - 4.3.3. [`duralade entity tick`](#433-duralade-entity-tick)
     - 4.3.4. [`duralade entity describe`](#434-duralade-entity-describe)
     - 4.3.5. [`duralade entity view`](#435-duralade-entity-view)
     - 4.3.6. [`duralade entity invoke-func`](#436-duralade-entity-invoke-func)
     - 4.3.7. [`duralade entity invoke-func-noblock`](#437-duralade-entity-invoke-func-noblock)
     - 4.3.8. [`duralade entity complete-extern`](#438-duralade-entity-complete-extern)
     - 4.3.9. [`duralade entity cancel`](#439-duralade-entity-cancel)
     - 4.3.10. [`duralade entity cancel-func`](#4310-duralade-entity-cancel-func)
     - 4.3.11. [`duralade entity checkpoint`](#4311-duralade-entity-checkpoint)
     - 4.3.12. [`duralade entity terminate`](#4312-duralade-entity-terminate)
     - 4.3.13. [`duralade entity shell`](#4313-duralade-entity-shell)
     - 4.3.14. [`duralade entity replay`](#4314-duralade-entity-replay)
     - 4.3.15. [`duralade entity inspect-stack`](#4315-duralade-entity-inspect-stack)
     - 4.3.16. [`duralade project test`](#4316-duralade-project-test)
   - 4.4. [Debug Operations](#44-debug-operations)
   - 4.5. [Deterministic Execution](#45-deterministic-execution)
   - 4.6. [Blocking and Concurrency](#46-blocking-and-concurrency)
   - 4.7. [Values and Memory Management](#47-values-and-memory-management)
   - 4.8. [Runtime Failure Model](#48-runtime-failure-model)
5. [History and State](#5-history-and-state)
   - 5.1. [Overview](#51-overview)
   - 5.2. [Event Types](#52-event-types)
   - 5.3. [System Externs and Entity Ref Operations](#53-system-externs-and-entity-ref-operations)
   - 5.4. [State Store](#54-state-store)
6. [Debugging](#6-debugging)

## 1. Overview

1. The Duralade engine provides both a command-line interface (CLI) and a programmatic API.
   - The CLI is a thin wrapper around the engine's programmatic API.
   - All CLI functionality is available programmatically through the Rust library.
   - 💭 Why both? The CLI provides an accessible interface for humans, while the programmatic API enables integration
     and automation.
1. The engine design reveals core capabilities of the Duralade runtime.
   - How code is compiled and bundled for deployment.
   - How entities are spawned and executed.
   - How execution state and history are managed.
   - How debugging and time-travel features are implemented.
1. All CLI commands support a `--codec` option for state encoding/encryption.
   - Applies to all state serialization and deserialization operations.
   - 💭 Why "codec"? Supports encryption, compression, or other encoding schemes, not just encryption.
   - ❓ Configuration format TBD (URI-based with support for HTTP endpoints, KMS providers, key material, etc.).
   - ❓ Protocol specification for codec endpoints TBD.

## 2. Project Structure

### 2.1. Overview

1. A Duralade project is a directory containing a `duralade.toml` configuration file and source code.
1. The project configuration defines the project name, source locations, dependencies, and other metadata.
1. Directory structure determines module hierarchy within the project namespace.

### 2.2. Project Configuration

1. The `duralade.toml` file defines an explicit project.
   - Projects are required for dependency management and cross-project imports.
   - When no `duralade.toml` is found, the directory is treated as an **implicit project**:
     - The project root name is `_` (underscore), a reserved sentinel that cannot appear in import paths.
     - The source root is the directory itself (equivalent to `source_root = "."`).
     - All `.dl` files in the directory (and subdirectories) are discovered as modules.
     - Multi-file modules work the same as in explicit projects.
     - Entity types omit the root prefix: `--entity money_transfer` resolves to `money_transfer.dl`.
       Dots translate to subdirectories: `--entity admin.roles` resolves to `admin/roles.dl`.
     - In persisted events, entity types are stored fully qualified with the `_.` prefix
       (e.g., `_.money_transfer`). The CLI adds this prefix transparently.
     - Only stdlib imports are available - `_` cannot be referenced in import statements.
     - No tests root is configured.
1. The `[project]` section defines basic project metadata.
   - `name` (required) - The project name, which becomes the root namespace for all modules.
   - `source_root` (optional) - The directory containing source files, defaults to `src/`.
   - `tests_root` (optional) - The directory containing test files, defaults to `test/`.
   - Example:
     ```toml
     [project]
     name = "my_project"
     ```
   - ❓ Version field TBD - may be derived from git tags instead of explicit toml field.
   - ❓ Multiple source/test roots TBD - should projects support multiple source_root or tests_root directories?
   - ❓ Implied imports TBD - the stdlib provides a default set of implied imports (currently just `duralade.error`).
     Projects can opt out with `stdlib_prelude = false`. Open questions:
     - Should projects be able to define their own additional implied imports?
     - Is "prelude" the right term, or something like "implied_imports" / "auto_imports"?
     - If custom implied imports are supported, what's the syntax? (e.g., `implied_imports = ["my_project.common"]`)
1. The optional `[build]` section specifies how to build native libraries from source.
   - Omit this section if project has no natives and source builds are allowed.
   - `type` (required) - Build system type: `"cargo"`, `"make"`, or `"none"`.
   - `type = "none"` indicates source builds are not supported (must distribute as `.dlb`).
   - Example for Rust natives:
     ```toml
     [build]
     type = "cargo"
     ```
   - Example for bundle-only distribution:
     ```toml
     [build]
     type = "none"
     ```
   - 💭 Why specify build system? Enables automatic building of dependencies with natives during source builds.
   - ❓ Build system details TBD (custom scripts, output paths, environment variables, cross-compilation).
   - ❓ Development mode TBD - how to reference debug build outputs during development (e.g., `cargo build` produces
     `target/debug/` not `lib/{target}/`).

### 2.3. Directory Layout

1. The source root directory contains the project's source files.
   - Source files have the `.dl` extension.
   - The source root defaults to `src/`.
   - Example: For project `my_project`, source files live in `src/`.
1. Directory structure within the source root maps directly to module hierarchy.
   - File `src/user.dl` defines module `{project_name}.user`.
   - File `src/admin/roles.dl` defines module `{project_name}.admin.roles`.
   - Subdirectory nesting creates nested module names.
1. The tests root directory contains test files in the `test` module hierarchy.
   - Test files are separate from the main project modules.
   - Example: `test/user_test.dl` defines module `test.user_test`.
   - Tests can only access `out` items from the main project by default.
   - 💭 Why separate modules? Enforces testing through public interfaces and prevents test code from polluting the
     project modules.
1. The `lib/` directory contains native shared libraries.
   - Organized by target triple: `lib/{target}/{project}.{ext}`
   - Target uses Rust/LLVM/GNU format (e.g., `x86_64-unknown-linux-gnu`)
   - Extension: `.so` (Linux), `.dll` (Windows), `.dylib` (macOS)
   - Example: `lib/x86_64-unknown-linux-gnu/my_project.so`

### 2.4. Visibility and Access Control

1. Underscore-prefixed names indicate project-internal visibility.
   - Module names: `_internal.dl` creates module `{project_name}._internal`.
   - Construct names: `data _helper { ... }` creates a project-internal type.
   - Project-internal items are visible within the project but not to external dependencies.
   - Project-internal items still require explicit imports within the project.
   - 💭 Why underscore prefix? Provides a visual convention for internal implementation details without requiring
     language-level access modifiers.
1. Test modules can access project-internal items using the `@test.internals_visible` annotation.
   - Applied at file level on test module files.
   - Accepts an array of specific module names that the test needs to access.
   - No wildcards or patterns are supported.
   - Example:

     ```
     @test::internals_visible(modules = ["my_project.internal", "my_project._helper"])

     import my_project.internal
     import my_project._helper
     ```

### 2.5. Dependencies

1. Dependencies are declared in the `[dependencies]` section of `duralade.toml`.
1. The TOML key for each dependency becomes its import alias.
   - Example: `auth = { ... }` allows `import auth.user`.
1. Four dependency types are supported.
1. **GitHub dependencies** reference releases with optional bundle assets.
   - Syntax: `name = { github = "org/repo", version = "1.2.3" }`
   - Resolution order for version `1.2.3` (git tag `v1.2.3`):
     1. Try platform-specific bundle: `{name}-{target}.dlb` from release assets
     2. Try multi-platform bundle: `{name}.dlb` from release assets
     3. Fall back to git clone and source build if allowed by remote project
     4. Error if no bundles found and remote project has `[build] type = "none"`
   - Target triple uses Rust/LLVM/GNU format (e.g., `x86_64-unknown-linux-gnu`).
   - 💭 Why fallback to source? Projects without natives don't need bundles, avoids timing window when release created
     before assets uploaded.
   - Example:
     ```toml
     [dependencies]
     auth = { github = "someorg/duralade-auth", version = "1.2.3" }
     ```
1. **Git dependencies** specify a repository URL for source builds.
   - Syntax: `name = { git = "url", version = "1.2.3" }`
   - Version selectors: `version` (semver with `v` prefix), `tag`, `branch`, `commit`
   - Optional `path` field for monorepo subdirectories
   - Always builds from source (never looks for bundles)
   - 💭 Why separate from github? Supports non-GitHub hosts and forces source builds when desired.
   - Example:
     ```toml
     [dependencies]
     utils = { git = "https://gitlab.com/org/utils", branch = "main" }
     ```
1. **Local path dependencies** point to source project directories.
   - Syntax: `name = { path = "../local-lib" }`
   - Path is relative to project root
   - Target must be a valid Duralade project with `duralade.toml`
1. **Local bundle dependencies** point to bundle files or directories.
   - Syntax: `name = { bundle_path = "path" }`
   - If path is a file: loads that specific `.dlb` file
   - If path is a directory: resolves `{name}-{target}.dlb` or `{name}.dlb`
   - Example:
     ```toml
     auth = { bundle_path = "../bundles/auth.dlb" }
     vendor = { bundle_path = "../vendor" }  # looks for vendor-{target}.dlb
     ```
1. **Stdlib dependency** is implicit but can be overridden.
   - The `duralade` stdlib is available by default without explicit declaration
   - Defaults to version statically linked into the runtime
   - Can be explicitly specified: `duralade = { github = "duralade/stdlib", version = "1.0.0" }`
1. Open questions about dependency management.
   - ❓ Should `name_override = true` be required when alias doesn't match dependency's project name?
   - ❓ Built-in "shading" support for dependency renaming/relocation/vendoring?
   - ❓ Test-only dependencies - should there be a `[dev-dependencies]` or similar section?
   - ❓ Optional dependencies and feature flags - how to handle optional deps and runtime features (like Cargo
     features)?
   - ❓ Dependency caching location TBD (e.g., `~/.duralade/cache/`).

## 3. Build System

### 3.1. Bundle Format

1. Duralade bundles (`.dlb` files) are ZIP archives of project directories.
   - Contains `duralade.toml`, source files, and `lib/` directory (if natives present).
   - Structure identical to project layout (see Section 2.3).
1. Bundle naming depends on native library presence.
   - No natives: `{project}.dlb`
   - Platform-specific natives: `{project}-{target}.dlb` (e.g., `auth-x86_64-unknown-linux-gnu.dlb`)
   - Multi-platform natives: `{project}.dlb` with multiple `lib/{target}/` directories
1. ❓ Bundle extraction and caching TBD (temporary directory, persistent cache, in-memory access).
1. ❓ Version compatibility and metadata TBD (bundle format version, minimum engine version).

### 3.2. Shared Libraries

1. Native functions are implemented via shared libraries loaded at runtime.
   - Platform-specific formats: `.dll` (Windows), `.so` (Linux), `.dylib` (macOS).
1. Shared libraries are discovered and loaded based on project configuration.
   - Libraries can be bundled with projects or referenced externally.
   - 💭 Why shared libraries? Enables implementation in any language, optimized performance for system operations, and
     language-agnostic interop.
1. Native function symbols follow a naming convention based on qualified names.
   - Format: `duralade_{qualified_name}` where dots are replaced with underscores.
   - Example: `native my_project.http.request` → symbol `duralade_my_project_http_request`
   - 💭 Why qualified names? Prevents symbol collisions when multiple projects are loaded.
1. All native functions share a uniform C ABI signature.
   - Functions accept a Duralade invocation context as an opaque handle.
   - The context provides C helper functions to access `in` fields and set `out` fields.
   - Helper functions allow zero-copy access to primitive arrays and strings when possible.
1. The engine provides a standard C header and optional Rust SDK for native implementation.
   - Helper functions abstract the invocation context and value manipulation.
   - 💭 Why opaque handles? Allows engine flexibility in internal representation while maintaining stable ABI.
1. Shared libraries are discovered in the project's `lib/{target}/` directory.
   - Target triple matches the current runtime platform (obtained at engine compile time).
   - One shared library per project: `{project_name}.{ext}` (e.g., `my_project.so`)
   - Example: For project `auth` on Linux x64, loads `lib/x86_64-unknown-linux-gnu/auth.so`
1. Libraries are loaded eagerly when the project is first accessed.
   - All native symbols for the project are resolved at load time.
   - Missing symbols for declared `native` functions result in a project load error.
   - 💭 Why eager? Fails fast on missing natives, avoids runtime surprises during execution.
1. Libraries may optionally export a `duralade_init` symbol for one-time initialization.
   - Called once immediately after library load, before any native functions.
   - Useful for library-level setup (connection pools, global state, etc.).
1. ❓ Memory management conventions TBD (ownership transfer, reference counting, allocation responsibilities).
1. ❓ Error handling mechanism TBD (panics, return codes, out! fields).
1. ❓ Thread safety requirements TBD (must natives be reentrant?).

## 4. Execution Model

### 4.1. Overview

TODO: Write after completing subsections 4.2-4.5.

### 4.2. Entity Lifecycle

1. Entities exist in one of two states: running or completed.
1. Entities are created via the spawn operation with a unique identifier.
   - Spawn runs an initial tick by default, executing `init` and starting `run`.
   - Spawn can optionally skip the initial tick to allow queueing events before first execution.
   - The pattern spawn-without-tick, add-event, tick enables pre-run event queuing.
   - 💭 Why pre-run events? Allows events to be queued before `run` starts, avoiding data race conditions.
1. Entity initialization occurs during the first tick.
   - The `init` function runs first if present.
   - Any queued invoke events are processed next, each starting as a coroutine.
   - The `run` function starts as a coroutine if present.
   - All steps occur within a single tick; invokes do not block `run` from starting.
1. Subsequent ticks continue execution from where it yielded.
   - The engine replays the event log to restore execution state.
   - Execution continues until all coroutines yield waiting for external stimulus.
1. Entities complete normally when `run` returns or via external termination.
   - Normal completion: `run` returns with `out` fields which can include error information.
   - Terminated completion: External terminate command stops execution without running code.
   - Both result in an EntityCompleted event.
1. Entity-level `out` fields and `run` `out` fields serve different purposes.
   - Entity `out` fields: Live-accessible state, queryable anytime including after completion.
   - `run` `out` fields: Final return values when entity completes normally.
   - ❓ Language specification update required: Section 8.5 needs to allow `out` fields in `run`.
1. Local entities follow the same lifecycle but execute within their parent entity's tick.
   - Created via direct construction (e.g., `my_entity(args)`).
   - No entity identifiers, not externally referenceable.

### 4.3. Core Operations

#### 4.3.1. Overview

1. Core operations control entity execution through CLI commands, HTTP API, and Rust library.
   - All interfaces provide equivalent functionality with similar semantics.
   - This section documents the CLI interface; HTTP and Rust APIs follow the same patterns.
1. Most entity commands require `--code <source>` to specify the code source.
   - Code is needed for any operation that reconstructs an entity instance (spawn, tick, view, complete-extern, invoke-func, cancel, cancel-func, terminate, replay, inspect).
   - Read-only metadata commands (describe) do not require code.
   - Defaults to `.` (current directory).
   - Supported formats:
     - Local paths: `/path/to/project`, `bundle.dlb`, `script.dl`
     - Git repositories: `git+https://github.com/org/repo?version=1.2.3`
     - Git version selectors: `version` (semver), `tag`, `branch`, `commit` (same as Section 2.5)
     - Monorepo subdirectories: append `&path=sub/dir` to Git URLs
1. All entity commands accept `--current-time <timestamp>` for controlling time.
   - Accepts absolute timestamps or relative durations (e.g., `+7d` for 7 days after last recorded time).
   - Defaults to wall-clock now.
   - Must be >= last time recorded in state (errors otherwise).
   - Time is recorded with state events, not on tick itself.
   - Does not auto-complete timer externs - those must be explicitly completed.

#### 4.3.2. `duralade entity spawn`

```
duralade entity spawn [OPTIONS]

Required:
  --entity <type>        Entity type (module-qualified: my_project.user)
  --id <id>              Entity identifier

State:
  --state <uri>          State store (default: ./duralade-state.json)

Behavior:
  --no-tick              Skip initial tick (default: runs tick)
  --dry-run              Show what would happen without writing

Input Arguments:
  --in <data>            Entity arguments (default: {})
  --in-file <path>       Read arguments from file
  --in-format <fmt>      Format override (default: infer from file extension, else json)
```

1. The spawn operation creates a new entity with a unique identifier in the state store.
1. The entity type can reference any module in the code source or its dependencies.
   - Example: `my_project.user`, `auth.service`, `workers.processor`
1. Entity arguments must be serializable (primitives, arrays, maps, data types, named function references).
   - Function closures cannot be serialized; entities with closure `in` fields are local-only.
1. Spawn runs an initial tick by default unless `--no-tick` is specified.
   - See Section 4.2 for entity initialization during first tick
1. Examples:
   - `duralade entity spawn --entity my_project.user --id user-123 --in '{"name": "Alice"}'`
   - `duralade entity spawn --entity worker.task --id t1 --state ./workers.json --code git+https://github.com/org/workers?version=2.0.0 --in-file args.json --no-tick`

#### 4.3.3. `duralade entity tick`

```
duralade entity tick [OPTIONS]

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --id <id>              Entity to tick (optional - ticks all if omitted)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing
```

1. The tick operation executes entities until all coroutines yield waiting for external stimulus.
1. With `--id`, ticks a single entity. Without `--id`, ticks all entities that can make progress.
   - Entities may cause other entities to become tickable (e.g., completing an extern that unblocks another entity). Tick without `--id` continues until no more progress can be made, which may tick individual entities multiple times.
1. Tick follows a three-phase process.
   - Replay: Read event log from state and replay to restore execution state.
   - Execute: Run all Duralade code until every coroutine yields.
   - Persist: Add new events to state and write back.
1. New events generated during tick include DuraladeExternInvoke, DuraladeFuncComplete, and DuraladeEntityComplete.
1. State is read from and written to the same store unless `--state-out` is specified.
1. ❓ Mechanism for outputting only new events from tick (not entire state) TBD.
1. ❓ Deadlock timeout configuration TBD (timeout when all coroutines are blocked waiting on each other).
1. ❓ Event grouping in state format to track which events came from which tick TBD (see Section 5).
1. Example: `duralade entity tick --id user-123`
1. Example: `duralade entity tick --state ./workers.json --id worker-1 --state-out stdout://`

#### 4.3.4. `duralade entity describe`

```
duralade entity describe [OPTIONS]

Required:
  --id <id>              Entity identifier

State:
  --state <uri>          State store (default: ./duralade-state.json)

Display Options:
  --get-out <field>      Include specific out field (can repeat)
  --get-out-all          Include all out fields
  --no-result            Omit run function result (for completed entities)

Output:
  --out <target>         Where to write result (default: stdout://)
  --out-format <fmt>     Format for result (default: json)
```

1. Describes the entity's current state and completion information.
1. For running entities, shows state as "running".
1. For completed entities, shows completion type (normal or terminated).
   - Normal completion includes `run` function's `out` fields unless `--no-result` is specified.
   - Terminated completion has no result.
1. Entity-level `out` fields are included based on `--get-out` or `--get-out-all`.
   - `--get-out <field>` can be repeated for multiple specific fields.
   - `--get-out-all` includes all entity-level `out` fields.
   - If both are specified, `--get-out-all` takes precedence.
1. Example: `duralade entity describe --id worker-123`
1. Example: `duralade entity describe --id worker-123 --get-out status --get-out progress`
1. Example: `duralade entity describe --id worker-123 --get-out-all --no-result`
1. ❓ Option to show pending externs and funcs (e.g., `--pending`) TBD.

#### 4.3.5. `duralade entity view`

```
duralade entity view [OPTIONS]

Required:
  --code <source>        Code source directory (default: ".")
  --id <id>              Entity identifier

One of (mutually exclusive):
  --func <name>          View function to invoke
  --field <name>         Specific out field to read (can repeat)
  --field-all            Read all out fields

Input Arguments (only with --func):
  --in <data>            Function arguments (default: {})
  --in-file <path>       Read arguments from file
  --in-format <fmt>      Format override (default: infer from file extension, else json)

State:
  --state <uri>          State store (default: ./duralade-state.json)

Optional:
  --after-event-num <n>  Truncate events to this number before replaying

Output:
  --out <target>         Where to write result (default: stdout://)
  --out-format <fmt>     Format for result (default: json)
```

1. Read-only command: invokes view function OR reads entity out fields (mutually exclusive).
1. Requires code to replay the entity and evaluate view functions or reconstruct out field values.
1. With `--func`: Replays entity, then executes view function and returns its out fields.
   - ❓ Support for `--eval` alternative to `--func` for evaluating arbitrary expressions TBD.
1. With `--field` or `--field-all`: Replays entity and reads entity out fields.
   - Multiple fields returned as data structure; single field returns just the value.
1. `--after-event-num` enables time-travel: truncates the event log to events with `num <= n`, then replays. Useful for inspecting entity state at a past point.
1. Example: `duralade entity view --id worker-123 --func get_status`
1. Example: `duralade entity view --id worker-123 --field status --field progress`
1. Example: `duralade entity view --id worker-123 --field-all --after-event-num 5`

#### 4.3.6. `duralade entity invoke-func`

```
duralade entity invoke-func [OPTIONS]

Required:
  --id <id>              Entity identifier
  --func <name>          Function name to invoke

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing

Input Arguments:
  --in <data>            Function arguments (default: {})
  --in-file <path>       Read arguments from file
  --in-format <fmt>      Format override (default: infer from file extension, else json)

Optional:
  --request-id <id>      Request identifier (default: auto-generated)
```

1. Adds a DuraladeFuncInvoke event to the entity's state.
1. The function will be invoked during the next tick.
   - Blocking functions run as coroutines; non-blocking functions execute inline.
1. Used for spawn-time invocations and external function calls.
   - Spawn-time: `spawn --no-tick`, `invoke-func`, then `tick`.
1. The tick generates a DuraladeFuncComplete event when the function returns.
1. Request ID identifies this invocation for later reference (e.g., `cancel-func`). Auto-generated if not provided.
1. Example: `duralade entity invoke-func --id w1 --func configure --in '{"mode": "fast"}'`
1. Example: `duralade entity invoke-func --id worker --func process --in-file item.json --request-id req-123`

#### 4.3.7. `duralade entity invoke-func-noblock`

```
duralade entity invoke-func-noblock [OPTIONS]

Required:
  --id <id>              Entity identifier
  --func <name>          Noblock function name to invoke

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing

Input Arguments:
  --in <data>            Function arguments (default: {})
  --in-file <path>       Read arguments from file
  --in-format <fmt>      Format override (default: infer from file extension, else json)

Output:
  --out <target>         Where to write result (default: stdout://)
  --out-format <fmt>     Format for result (default: json)
```

1. Shortcut for `invoke-func` + `tick` for noblock functions.
1. Adds a DuraladeFuncInvoke event, runs the tick, and returns the function's out fields.
1. Noblock functions can mutate entity state.
1. Example: `duralade entity invoke-func-noblock --id worker --func update_config --in '{"timeout": 30}'`
1. Example: `duralade entity invoke-func-noblock --id worker --func process --in-file data.json --out result.json`

#### 4.3.8. `duralade entity complete-extern`

```
duralade entity complete-extern [OPTIONS]

Required:
  --id <id>              Entity identifier
  --invoke-num <num>     Event number of the DuraladeExternInvoke to complete

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing

Result Data:
  --result <data>        Extern result (default: {})
  --result-file <path>   Read result from file
  --result-format <fmt>  Format override (default: infer from file extension, else json)
```

1. Adds a DuraladeExternComplete event to the entity's state.
1. The invoke number must reference a DuraladeExternInvoke event from a previous tick.
1. Example: `duralade entity complete-extern --id user-1 --invoke-num 42 --result '{"status": 200}'`
1. Example: `duralade entity complete-extern --id worker --invoke-num 43 --result-file result.json`

#### 4.3.9. `duralade entity cancel`

```
duralade entity cancel [OPTIONS]

Required:
  --id <id>              Entity identifier

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing
```

1. Adds a DuraladeEntityCancel event to the entity's state.
1. Cancellation is cooperative - the entity decides how to respond during execution.
1. Only one cancel request is allowed per entity.
   - Subsequent cancel requests on the same entity will error.
1. ❓ Support for cancel reason or message TBD (e.g., `--reason "timeout"`).
1. Example: `duralade entity cancel --id worker-123`

#### 4.3.10. `duralade entity cancel-func`

```
duralade entity cancel-func [OPTIONS]

Required:
  --id <id>              Entity identifier
  --request-id <id>      Request ID of the DuraladeFuncInvoke to cancel

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing
```

1. Adds a DuraladeFuncCancel event to the entity's state.
1. The request ID must reference an in-flight DuraladeFuncInvoke (one without a corresponding DuraladeFuncComplete).
1. Cancellation is cooperative - the func's code decides how to respond.
1. Example: `duralade entity cancel-func --id worker --request-id req-123`

#### 4.3.11. `duralade entity checkpoint`

```
duralade entity checkpoint [OPTIONS]

Required:
  --id <id>              Entity identifier

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing

Compaction:
  --no-compact           Keep all events instead of removing completed ones (default: compact)
```

1. Adds a Checkpoint event capturing current execution state.
1. By default, removes events that are no longer needed (compaction).
   - Removes completed invoke pairs (DuraladeExternInvoke + DuraladeExternComplete, DuraladeFuncInvoke +
     DuraladeFuncComplete).
   - Keeps incomplete invokes (request without completion).
   - Keeps other events (DuraladeEntityCancel, etc.).
1. With `--no-compact`, preserves all event history.
1. Example: `duralade entity checkpoint --id worker-123`
1. Example: `duralade entity checkpoint --id worker-123 --no-compact`

#### 4.3.12. `duralade entity terminate`

```
duralade entity terminate [OPTIONS]

Required:
  --id <id>              Entity identifier

State:
  --state <uri>          State store (default: ./duralade-state.json)
  --state-out <uri>      Write to different store
  --dry-run              Show what would happen without writing
```

1. Adds a DuraladeEntityComplete event marking the entity as terminated.
1. Forces the entity into completed state without running any code.
   - Does not execute defer blocks or cleanup code.
   - Does not produce `run` function out fields.
1. Used for operator intervention or emergency stops.
1. ❓ Support for termination reason or message TBD (e.g., `--reason "manual stop"`).
1. Example: `duralade entity terminate --id worker-123`

#### 4.3.13. `duralade entity shell`

```
duralade entity shell [OPTIONS]

State:
  --state <uri>          State store (default: ./duralade-state.json)

Persistence:
  --no-auto-persist      Defer writes, use explicit `persist` command
  --read-only            Prevent any state modifications
```

1. The shell operation opens an interactive session with the state store loaded in memory.
   - 💭 Why? Avoids repeated state deserialization, enabling fast iteration during development and debugging.
1. All entity commands from Section 4.3 are available without the `duralade entity` prefix.
   - The `--state` parameter is omitted since state is already loaded.
   - Example: `tick --id worker-123` instead of `duralade entity tick --id worker-123`
   - Example: `view --id worker-123 --func get_status`
1. Shell-specific commands are available.
   - `reload` - Re-read state from the store, discarding in-memory changes.
   - `persist` - Explicitly save state (when `--no-auto-persist` is enabled).
   - `exit` or `quit` - Exit the shell session.
1. By default, state is persisted after each mutation.
   - Use `--no-auto-persist` to defer writes and use explicit `persist` commands.
   - `--read-only` prevents any state modifications.
1. ❓ Shell interface improvements TBD (command history, tab completion, syntax highlighting, TUI mode with richer
   interaction).
1. ❓ Integration with spawn TBD (`spawn --shell` to immediately enter shell).
1. Example: `duralade entity shell --state ./workers.json`

#### 4.3.14. `duralade entity replay`

```
duralade entity replay [OPTIONS]

Required:
  --code <source>        Code source directory (default: ".")
  --id <id>              Entity identifier

State:
  --state <uri>          State store (default: ./duralade-state.json)
```

1. Diagnostic, read-only command: replays an entity from its event log without persisting anything.
1. Confirms the entity can replay without faulting under the current code.
   - Useful after code changes to verify existing entities are still compatible.
1. Prints the replay outcome: "replay ok: entity completed" or "replay ok: entity blocked".
1. If replay produces a fault, the command fails with the fault message.
1. Example: `duralade entity replay --id worker-123`

#### 4.3.15. `duralade entity inspect-stack`

```
duralade entity inspect-stack [OPTIONS]

Required:
  --code <source>        Code source directory (default: ".")
  --id <id>              Entity identifier

State:
  --state <uri>          State store (default: ./duralade-state.json)

Optional:
  --after-event-num <n>  Truncate events to this number before replaying

Output:
  --out <target>         Where to write result (default: stdout://)
  --out-format <fmt>     Format for result (default: json)
```

1. Diagnostic, read-only command: replays entity and displays the call stack of each active coroutine.
1. Output is a JSON object with:
   - `last_event_num`: the highest event number in the replayed log.
   - `coroutines`: array of coroutine info objects, each with:
     - `id`: coroutine identifier.
     - `source_event`: event number of the FuncInvoke that created this coroutine (null for the run coroutine).
     - `stack`: array of resolved stack frames, bottom-to-top. Each frame has:
       - `module`: fully qualified module path (e.g. `myapp.worker`).
       - `construct`: declaration name (e.g. `worker`).
       - `member`: member name if inside a member function (omitted otherwise).
       - `file`: source file path (omitted if unavailable).
       - `line`: 1-indexed line number of the call site (omitted if unavailable).
1. `--after-event-num` enables inspecting stacks at a past point in the event log.
1. Example: `duralade entity inspect-stack --id worker-123`

#### 4.3.16. `duralade project test`

```
duralade project test [OPTIONS] [FILTER]

Options:
  --code <source>        Code source directory (default: ".")

Arguments:
  [FILTER]               Only run tests whose qualified name contains this substring
```

1. Discovers and runs all `@test` functions in the project's test modules.
   - Test modules are loaded from the project's tests root (default: `test/`).
   - Each `@test` function runs as an isolated entity in a fresh in-memory state store.
   - Tests that complete normally pass; tests that early-return via `!` fail with the error message.
1. The optional filter argument selects tests by qualified name substring.
   - Qualified names follow the format `{root}.{module_segments}::{func_name}`.
   - Example: `duralade project test spawn` runs all tests containing "spawn" in their qualified name.
1. Output streams test results as they complete.
   - Each test prints its qualified name, PASS/FAIL status, and duration.
   - A summary line reports total passed, failed, and skipped counts.
   - The command exits with a non-zero code if any tests fail.
1. Example: `duralade project test --code ./my_project`
1. Example: `duralade project test --code ./my_project user_test`

### 4.4. Debug Operations

1. The primary debugging tools are replay-based: replay the entity from its event log with current code, then inspect.
   - `replay` (§4.3.14) confirms an entity can replay without faulting.
   - `view` (§4.3.5) with `--after-event-num` enables time-travel inspection of out fields and view functions.
   - `inspect-stack` (§4.3.15) shows call stacks of active coroutines at any point in the event log.
1. All debug operations are read-only - they do not modify the state store.
1. TODO: Interactive debugging (breakpoints, stepping, expression evaluation).

### 4.5. Deterministic Execution

1. TODO: Extern implementation mechanisms
1. TODO: Memoization and replay behavior
1. ❓ All externs as RPCs at the engine level - should the engine expose a generic RPC interface for extern invocations,
   allowing arbitrary implementations (local functions, HTTP endpoints, message queues, etc.)?
1. ❓ Author-level extern HTTP definitions - should authors be able to declare externs with HTTP endpoint configurations
   directly in code?

### 4.6. Blocking and Concurrency

1. TODO: Local entity construction and execution semantics
1. TODO: Entity reference types and `spawn` builtin
1. TODO: Blocking behavior with `->` operator for entity refs
1. TODO: Task semantics and cooperative multitasking
1. TODO: Get spawn + entity ref stuff right - how does spawn work, what does it return, how do entity refs behave, what
   operations are available on them (.id, .cancel(), .terminate()?), how does blocking work, what's the difference
   between local entities and spawned entities

### 4.7. Values and Memory Management

1. All values behave as references from the user's perspective.
   - Assignment copies the reference, not the data: `b := a` makes both refer to the same value.
   - 💭 Why? Consistent mental model - no value vs reference distinction. Works like JavaScript.
1. Immutable types (`int`, `float`, `bool`, `nil`, `str`) may be implemented as copy-by-value.
   - Since they cannot be mutated, reference vs copy is indistinguishable to the user.
   - Strings may use copy-on-write or interning internally.
1. Mutable types (`data`, `list`, `map`, function closures) are shared by reference.
   - Mutations are visible through all references to the same value.
1. Entity references hold an identifier, not inline state. Entity state lives in a separate runtime store.
1. Non-primitive values live on a value heap. Variables hold small handles (value IDs) into the heap.
   - The heap is serialized at checkpoints. Loaded modules (ASTs, types) are static and NOT serialized per tick.
1. Memory is managed via reference counting with type-directed cycle detection.
   - Refcount reaches zero → immediate free.
   - Cycles are possible (e.g., `data Node { :node? }` referencing itself).
   - At load time, the type checker computes which types are **cycle-capable** - those whose field graph can transitively reference back to themselves.
   - Only cycle-capable values are tracked as **suspects** when their refcount decreases but doesn't reach zero.
   - Cycle collection (trial deletion) runs at tick boundaries, scoped to suspects only.
   - 💭 Why type-directed? Most types can't form cycles, so the suspects set is typically empty. Cycle detection is effectively free for most programs.
1. GC is invisible to user code. No finalizers or weak references. `defer` runs at scope exit, not GC time.

### 4.8. Runtime Failure Model

Three categories of runtime conditions:

#### Errors (user-reactable)

1. Errors are values via `out!` fields - normal control flow, entity continues executing.
1. Every foreseeable condition must be expressible as an error. If user code cannot react to a foreseeable condition,
   that is a language design bug.

#### Faults

1. A **fault** halts entity execution due to an environmental or data mismatch.
   - Extern result validation failure (missing required field, extra field, wrong type).
   - Replay divergence (code changed, recorded events don't match new execution path).
   - Missing non-nilable implicit (no `implicitly` binding in call chain).
   - Deserialization failure (stored value doesn't match expected type shape).
1. No in-language catch/recover mechanism. The `out!` path handles expected errors; faults represent unanticipated
   conditions. The durable execution model provides a better recovery path than in-code recovery: fix code or data,
   re-tick.
1. A fault is a **tick result, not entity state**. No new events are persisted on fault. The event log is unchanged,
   the entity remains "running" in the store, and re-ticking replays from scratch. Faults are idempotent - same code
   and event log produce the same fault.
   - 💭 Why not record faults in the event log? The event log records what happened - invocations, completions, spawn.
     A fault is what *didn't* happen: execution could not proceed. Recording it would require a "clear fault" operation
     before re-ticking, couple the log format to error reporting, and add schema complexity for something that belongs
     in operational telemetry (CLI output, dashboards, monitoring).
1. Tick result carries structured fault information (category, message, source location, execution trace).
   - ❓ Exact `FaultInfo` structure TBD.

#### Fault recovery

1. Recovery = fix inputs (code and/or event log), then re-tick.
1. **Code surgery**: fix the code, re-tick. Entity replays from event log with updated code. Primary and safest path.
1. **Event log surgery**: edit events, re-tick. Fix malformed extern results, remove divergence-causing events, correct
   bad values. Powerful but dangerous - the event log format is intentionally simple to make manual editing feasible.
1. Faults at different points after a fix are expected - iterate until the entity makes it through.

#### Internal errors

1. An **internal error** is a bug in the Duralade compiler or runtime - conditions that should be impossible if
   parser/loader/type-checker are correct (e.g., type mismatch at runtime, undefined variable, dangling heap ref).
1. Also halts the entity, but the message indicates a compiler bug, not a user-fixable problem.
1. Distinct error type from faults in the tick result so tooling can differentiate.

## 5. History and State

### 5.1. Overview

1. A **state store** holds all entities for a deployment. Entities within a store can reference each other via entity refs.
   - The store is the unit of scope - spawn creates entities in the same store.
   - The CLI uses `--state <uri>` to identify the store, `--id <entity_id>` to target a specific entity within it.
1. Each entity's state is an append-only event log.
   - The current execution state is derived by replaying all events in order.
   - Each tick appends new events to the log.
   - 💭 Why event log? Provides complete history for time-travel debugging, deterministic replay, and auditability.
1. Events serialize to multiple formats, with JSON as the default.
   - Format inferred from file extension or provided as a query parameter on the URI (e.g., `?format=json`). Default is JSON.
   - Fields marked `user_payload` can be encoded via `--codec` for encryption, compression, or other transformations
     (see Section 1).
1. Event schemas use Duralade type notation with primitive types (`int`, `str`, `bool`, `bytes`, `array[t]`,
   `map[key, value]`, nilability with `?`).
   - `bytes` represents binary data; serialized as base64 in JSON, as raw bytes in binary formats.
   - Actual serialization uses the format's native representation (JSON objects, binary structs, etc.).
1. Checkpoint events enable log compaction by capturing full execution state at a point in time.
   - Replay starts from the most recent Checkpoint; earlier events can be discarded.

### 5.2. Event Types

1. Common fields present in all events:
   - `type: str` - Event type identifier (e.g., "DuraladeEntityCancel").
   - `num: int` - Monotonically increasing event number (may have gaps).
   - `time: int` - Milliseconds since Unix epoch (UTC).
1. The first event in the log must be either `DuraladeEntityInvoke` or `DuraladeEntityCheckpoint`.
1. Event types defined in this section:
   - `DuraladeEntityCancel` - Cooperative cancellation request. **Written by user.**
     - `reason: user_payload[str]?` - Optional reason for cancellation.
   - `DuraladeEntityCheckpoint` - Snapshot of execution state for log compaction. **Written by user.**
     - `id: str` - Entity identifier.
     - `state: user_payload[bytes]` - Captured execution state at this point in time.
   - `DuraladeEntityComplete` - Entity finished execution. **Written by Duralade (normal completion) or user
     (terminated).**
     - `terminated: bool` - True if entity was terminated, false if completed normally.
     - `result: user_payload[map[str, any]]?` - Run function's out fields (only present for normal completion).
   - `DuraladeEntityInvoke` - Entity spawn event. **Written by user.**
     - `id: str` - Unique identifier for this entity.
     - `entity: str` - Fully qualified entity type name (e.g., "my_project.user").
     - `args: user_payload[map[str, any]]` - Entity constructor arguments (in/inout fields).
   - `DuraladeExternComplete` - Extern invocation completed. **Written by user.**
     - `invoke_num: int` - References the num of the DuraladeExternInvoke.
     - `result: user_payload[map[str, any]]` - Extern function's out fields.
   - `DuraladeExternInvoke` - Extern invocation requested by Duralade code. **Written by Duralade.**
     - `extern: str` - Fully qualified extern name: `<dot-delimited-module>::<extern>` (e.g., `my_project.payments::charge`, `duralade.entity::spawn`).
     - `args: user_payload[map[str, any]]` - Extern function's in/inout fields.
   - `DuraladePatchSelect` - Patch version selected during execution. **Written by Duralade.**
     - `version: str` - Qualified identifier of the selected patch version (e.g., "my_feature.v2").
   - `DuraladeFuncComplete` - Function invocation completed. **Written by Duralade.**
     - `invoke_num: int` - References the num of the DuraladeFuncInvoke.
     - `result: user_payload[map[str, any]]` - Function's out fields.
   - `DuraladeFuncCancel` - Function invocation cancelled. **Written by user.**
     - `invoke_num: int` - References the num of the DuraladeFuncInvoke.
     - `reason: str?` - Optional reason for cancellation.
   - `DuraladeFuncInvoke` - Function invocation requested. **Written by user.**
     - `id: str?` - Optional user-provided identifier for this function invocation.
     - `func: str` - Name of the function to invoke.
     - `args: user_payload[map[str, any]]` - Function's in/inout fields.

### 5.3. System Externs and Entity Ref Operations

1. System externs are extern invocations handled by the runtime rather than user code.
   - All system externs use the `duralade.*::` namespace (e.g., `duralade.entity::spawn`, `duralade.time::sleep`).
   - The interpreter treats them identically to user externs - ExternInvoke yields, ExternComplete resumes.
   - 💭 Why model as externs? Avoids new execution primitives. Event logs stay uniform.
1. Entity ref operations use the `duralade.entity::` namespace. From each entity's perspective, cross-entity interactions are externs (outbound) or funcs (inbound). The runtime routes between entities.
1. Entity ref API - operations available on an entity reference (`&T`):
   - `ref->func(args)` - call a member function. See `duralade.entity::call` below.
   - `ref->field` - read a field. See `duralade.entity::read` below.
   - `ref.result()` - wait for the entity's `run` to complete and return its result. See `duralade.entity::result` below.
   - `ref.cancel(reason?)` - request cooperative cancellation. See `duralade.entity::cancel` below.
   - `ref.terminate()` - force termination. See `duralade.entity::terminate` below.
   - `ref.id` - the entity instance identifier (`str`). Local, no extern.
   - ❓ `ref->*` - read all `out` fields at once. TBD.
   - ❓ `ref.domain` - the execution domain/location of the entity. Needed for serialization of entity refs (id + domain are sufficient to reconstruct a ref). Semantics and naming TBD.
1. `duralade.entity::spawn` - create an independent entity.
   - Caller: ExternInvoke `{ entity, id?, ... }` → ExternComplete `{ entity: &ref?, error? }`
   - Target: receives `DuraladeEntityInvoke` as its first event.
1. `duralade.entity::result` - wait for a spawned entity's `run` to complete.
   - Caller: ExternInvoke `{ entity_ref }` → ExternComplete `{ value?, error? }`
   - Target: FuncInvoke `{ func: "duralade.entity::result", args: { entity_ref } }` → FuncComplete `{ result }`
   - Only one `result` func is in flight on the target per entity ref - multiple waiters share it.
   - Once received, the result is memoized on the entity ref instance. Subsequent calls return it without a new extern or func.
   - If the target is already done when called, completes immediately without a FuncInvoke on the target.
   - The system cancels the `result` func when no references to the entity remain on the caller side. Requires a general func cancellation concept (see below).
   - ❓ Atomic spawn + result: callers may want to spawn and immediately wait. The entity_ref may not exist yet at that point. Need to decide if this is a combined operation or syntactic sugar over two sequential externs.
1. `duralade.entity::call` - member function call (`ref->method(args)`).
   - Caller: ExternInvoke `{ entity_ref, func, args }` → ExternComplete `{ result }`
   - Target: receives `DuraladeFuncInvoke` and produces `DuraladeFuncComplete` only for non-view funcs. View funcs execute without events.
1. `duralade.entity::read` - field read (`ref->field`).
   - Caller: ExternInvoke `{ entity_ref, field }` → ExternComplete `{ value }`
   - ❓ Whether field reads produce events on the target TBD (likely no).
1. `duralade.entity::cancel` - cooperative cancellation.
   - Caller: ExternInvoke `{ entity_ref, reason? }` → ExternComplete `{}`
   - Target: receives `DuraladeEntityCancel`.
   - Completes immediately - does not wait for target to stop.
   - ❓ Whether cancel should be noblock (no extern/yield) TBD.
1. `duralade.entity::terminate` - force termination.
   - Caller: ExternInvoke `{ entity_ref }` → ExternComplete `{}`
   - Target: receives `DuraladeEntityComplete { terminated: true }`.
   - Completes immediately.
   - ❓ Whether terminate should be noblock (no extern/yield) TBD.

### 5.4. State Store

1. The state store holds all entities for a deployment as a collection of entity event logs.
   - CLI: `--state <uri>` identifies the store, `--id <entity_id>` targets an entity within it.
   - URI: `./state.json` (implicit `file://`), `file:///path/to/state.json`, or database URIs (future).
1. **File backend** - a single file, top-level object keyed by entity ID, each value is an array of events.
   - Format inferred from file extension. Default is JSON. Override via query param (`?format=...`).
1. ❓ Database backend TBD.

## 6. Debugging

TODO: Stepping through execution, rewinding/time-travel debugging, breakpoints, state inspection.
