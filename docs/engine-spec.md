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
     - 4.3.10. [`duralade entity checkpoint`](#4310-duralade-entity-checkpoint)
     - 4.3.11. [`duralade entity terminate`](#4311-duralade-entity-terminate)
     - 4.3.12. [`duralade entity shell`](#4312-duralade-entity-shell)
   - 4.4. [Debug Operations](#44-debug-operations)
   - 4.5. [Deterministic Execution](#45-deterministic-execution)
   - 4.6. [Blocking and Concurrency](#46-blocking-and-concurrency)
5. [History and State](#5-history-and-state)
   - 5.1. [Overview](#51-overview)
   - 5.2. [Event Types](#52-event-types)
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

1. The `duralade.toml` file is required at the project root.
   - Single `.dl` files can be run without a project configuration.
   - Projects are required for dependency management and multi-file organization.
1. The `[project]` section defines basic project metadata.
   - `name` (required) - The project name, which becomes the root namespace for all modules.
   - `source_root` (optional) - The directory containing source files, defaults to `{project_name}/`.
   - `tests_root` (optional) - The directory containing test files, defaults to `tests/`.
   - Example:
     ```toml
     [project]
     name = "my_project"
     source_root = "src"
     tests_root = "tests"
     ```
   - 💭 Why default to project name? Follows Python convention where package name matches directory name, making the
     structure self-documenting.
   - ❓ Version field TBD - may be derived from git tags instead of explicit toml field.
   - ❓ Multiple source/test roots TBD - should projects support multiple source_root or tests_root directories?
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
   - The source root defaults to a directory matching the project name.
   - Example: For project `my_project`, the default source root is `my_project/`.
1. Directory structure within the source root maps directly to module hierarchy.
   - File `{source_root}/user.dl` defines module `{project_name}.user`.
   - File `{source_root}/admin/roles.dl` defines module `{project_name}.admin.roles`.
   - Subdirectory nesting creates nested module names.
1. The tests root directory contains test files in the `tests` module hierarchy.
   - Test files are separate from the main project modules.
   - Example: `tests/user_test.dl` defines module `tests.user_test`.
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
     @test.internals_visible(modules = ["my_project.internal", "my_project._helper"])

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
1. All entity commands accept `--code <source>` to specify the code source.
   - Code source determines which code version is used for execution.
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

State Output:
  --state <target>       Where to write state (default: <id>.duralade.json)
  --state-format <fmt>   Format override (default: infer from extension, else json)

Behavior:
  --no-tick              Skip initial tick (default: runs tick)
  --dry-run              Show what would happen without writing

Input Arguments:
  --in <data>            Entity arguments (default: {})
  --in-file <path>       Read arguments from file
  --in-format <fmt>      Format override (default: infer from file extension, else json)
```

1. The spawn operation creates a new entity with a unique identifier.
1. The entity type can reference any module in the code source or its dependencies.
   - Example: `my_project.user`, `auth.service`, `workers.processor`
1. State targets support URI schemes.
   - `file://` is implied for paths
   - `stdout://` writes to stdout
   - `db://` for database storage (future)
1. Format inference rules for state and input data.
   - If file extension is present, infer from extension
   - Otherwise default to JSON
   - Error if file path provided and format cannot be inferred
1. Entity arguments must be serializable (primitives, arrays, maps, data types, named function references).
   - Function closures cannot be serialized; entities with closure `in` fields are local-only.
1. Spawn runs an initial tick by default unless `--no-tick` is specified.
   - See Section 4.2 for entity initialization during first tick
1. Examples:
   - `duralade entity spawn --entity my_project.user --id user-123 --in '{"name": "Alice"}'`
   - `duralade entity spawn --entity worker.task --id t1 --code git+https://github.com/org/workers?version=2.0.0 --in-file args.json --no-tick`
   - `duralade entity spawn --entity service --id svc-1 --code bundle.dlb --state stdout:// --in '{}'`

#### 4.3.3. `duralade entity tick`

```
duralade entity tick [OPTIONS]

State I/O:
  --state <target>       Read/write state (required)
  --state-out <target>   Write to different target
  --state-format <fmt>   Format override
  --dry-run              Show what would happen without writing
```

1. The tick operation executes the entity until all coroutines yield waiting for external stimulus.
1. Tick follows a three-phase process.
   - Replay: Read event log from state and replay to restore execution state.
   - Execute: Run all Duralade code until every coroutine yields.
   - Persist: Add new events to state and write back.
1. New events generated during tick include InvokeExternRequest, InvokeFuncComplete, and EntityCompleted.
   - InvokeExternRequest events indicate side effects the entity needs executed externally.
   - InvokeFuncComplete events indicate a function invocation has completed.
   - EntityCompleted event indicates the entity has finished execution.
1. State is read from and written to the same target unless `--state-out` is specified.
   - With `--state-out`, the original state file is preserved.
1. ❓ Mechanism for outputting only new events from tick (not entire state) TBD.
1. ❓ Deadlock timeout configuration TBD (timeout when all coroutines are blocked waiting on each other).
1. ❓ Event grouping in state format to track which events came from which tick TBD (see Section 5).
1. Example: `duralade entity tick --state user-123.duralade.json`
1. Example: `duralade entity tick --state entities/worker-1.json --state-out stdout://`

#### 4.3.4. `duralade entity describe`

```
duralade entity describe [OPTIONS]

State Input:
  --state <target>       Read state from (required)
  --state-format <fmt>   Format override

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
1. Example: `duralade entity describe --state worker-123.json`
1. Example: `duralade entity describe --state worker-123.json --get-out status --get-out progress`
1. Example: `duralade entity describe --state worker-123.json --get-out-all --no-result`

#### 4.3.5. `duralade entity view`

```
duralade entity view [OPTIONS]

One of (mutually exclusive):
  --func <name>          View function to invoke
  --field <name>         Specific out field to read (can repeat)
  --field-all            Read all out fields

Input Arguments (only with --func):
  --in <data>            Function arguments (default: {})
  --in-file <path>       Read arguments from file
  --in-format <fmt>      Format override (default: infer from file extension, else json)

State Input:
  --state <target>       Read state from (required)
  --state-format <fmt>   Format override

Output:
  --out <target>         Where to write result (default: stdout://)
  --out-format <fmt>     Format for result (default: json)
```

1. Read-only command: invokes view function OR reads entity out fields (mutually exclusive).
1. With `--func`: Executes view function and returns its out fields.
   - ❓ Support for `--eval` alternative to `--func` for evaluating arbitrary expressions TBD.
1. With `--field` or `--field-all`: Reads entity out fields without executing code.
   - Multiple fields returned as data structure; single field returns just the value.
1. Example: `duralade entity view --func get_status --state worker-123.json`
1. Example: `duralade entity view --field status --field progress --state worker-123.json`

#### 4.3.6. `duralade entity invoke-func`

```
duralade entity invoke-func [OPTIONS]

Required:
  --func <name>          Function name to invoke

State I/O:
  --state <target>       Read/write state (required)
  --state-out <target>   Write to different target
  --state-format <fmt>   Format override
  --dry-run              Show what would happen without writing

Input Arguments:
  --in <data>            Function arguments (default: {})
  --in-file <path>       Read arguments from file
  --in-format <fmt>      Format override (default: infer from file extension, else json)

Optional:
  --id <id>              Request identifier (default: auto-generated)
```

1. Adds a DuraladeFuncInvoke event to the entity's state.
1. The function will be invoked during the next tick.
   - Blocking functions run as coroutines; non-blocking functions execute inline.
1. Used for spawn-time invocations and external function calls.
   - Spawn-time: `spawn --no-tick`, `invoke-func`, then `tick`.
1. The tick generates a DuraladeFuncComplete event when the function returns.
1. Request ID is auto-generated if not provided.
1. Example: `duralade entity invoke-func --func configure --state w1.json --in '{"mode": "fast"}'`
1. Example: `duralade entity invoke-func --func process --state worker.json --in-file item.json --id req-123`

#### 4.3.7. `duralade entity invoke-func-noblock`

```
duralade entity invoke-func-noblock [OPTIONS]

Required:
  --func <name>          Noblock function name to invoke

State I/O:
  --state <target>       Read/write state (required)
  --state-out <target>   Write to different target
  --state-format <fmt>   Format override
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
1. Example: `duralade entity invoke-func-noblock --func update_config --state worker.json --in '{"timeout": 30}'`
1. Example:
   `duralade entity invoke-func-noblock --func process --state worker.json --in-file data.json --out result.json`

#### 4.3.8. `duralade entity complete-extern`

```
duralade entity complete-extern [OPTIONS]

Required:
  --request-event <num>  Event number of the DuraladeExternInvoke to complete

State I/O:
  --state <target>       Read/write state (required)
  --state-out <target>   Write to different target
  --state-format <fmt>   Format override
  --dry-run              Show what would happen without writing

Result Data:
  --result <data>        Extern result (default: {})
  --result-file <path>   Read result from file
  --result-format <fmt>  Format override (default: infer from file extension, else json)
```

1. Adds a DuraladeExternComplete event to the entity's state.
1. The request event number must reference a DuraladeExternInvoke event from a previous tick.
1. Format inference follows the same rules as input arguments (see Section 4.3.2).
1. Example: `duralade entity complete-extern --request-event 42 --state user-1.json --result '{"status": 200}'`
1. Example: `duralade entity complete-extern --request-event 43 --state worker.json --result-file result.json`

#### 4.3.9. `duralade entity cancel`

```
duralade entity cancel [OPTIONS]

State I/O:
  --state <target>       Read/write state (required)
  --state-out <target>   Write to different target
  --state-format <fmt>   Format override
  --dry-run              Show what would happen without writing
```

1. Adds a DuraladeEntityCancel event to the entity's state.
1. Cancellation is cooperative - the entity decides how to respond during execution.
1. Only one cancel request is allowed per entity.
   - Subsequent cancel requests on the same entity will error.
1. ❓ Support for cancel reason or message TBD (e.g., `--reason "timeout"`).
1. ❓ `cancel-func` command TBD - cancel a specific in-flight function invocation by request ID. Note: not all runtimes
   may support this.
1. Example: `duralade entity cancel --state worker-123.json`

#### 4.3.10. `duralade entity checkpoint`

```
duralade entity checkpoint [OPTIONS]

State I/O:
  --state <target>       Read/write state (required)
  --state-out <target>   Write to different target
  --state-format <fmt>   Format override
  --dry-run              Show what would happen without writing

Compaction:
  --no-compact           Keep all events instead of removing completed ones (default: compact)
```

1. Adds a Checkpoint event capturing current execution state.
1. By default, removes events that are no longer needed (compaction).
   - Removes completed invoke pairs (InvokeExternRequest + InvokeExternComplete, InvokeFuncRequest +
     InvokeFuncComplete).
   - Keeps incomplete invokes (request without completion).
   - Keeps other events (CancelRequested, etc.).
1. With `--no-compact`, preserves all event history.
1. Example: `duralade entity checkpoint --state worker-123.json`
1. Example: `duralade entity checkpoint --state worker-123.json --no-compact`

#### 4.3.11. `duralade entity terminate`

```
duralade entity terminate [OPTIONS]

State I/O:
  --state <target>       Read/write state (required)
  --state-out <target>   Write to different target
  --state-format <fmt>   Format override
  --dry-run              Show what would happen without writing
```

1. Adds an EntityCompleted event marking the entity as terminated.
1. Forces the entity into completed state without running any code.
   - Does not execute defer blocks or cleanup code.
   - Does not produce `run` function out fields.
1. Used for operator intervention or emergency stops.
1. ❓ Support for termination reason or message TBD (e.g., `--reason "manual stop"`).
1. Example: `duralade entity terminate --state worker-123.json`

#### 4.3.12. `duralade entity shell`

```
duralade entity shell [OPTIONS]

State Input:
  --state <target>       Entity state to load (required)
  --state-format <fmt>   Format override

Persistence:
  --no-auto-persist      Defer writes, use explicit `persist` command
  --read-only            Prevent any state modifications
```

1. The shell operation opens an interactive session with an entity's state loaded in memory.
   - 💭 Why? Avoids repeated state deserialization, enabling fast iteration during development and debugging.
1. All entity commands from Section 4.3 are available without the `duralade entity` prefix.
   - The `--state` parameter is omitted since state is already loaded.
   - Example: `tick` instead of `duralade entity tick --state file.json`
   - Example: `invoke-view --func get_status` instead of
     `duralade entity invoke-view --func get_status --state file.json`
1. Shell-specific commands are available.
   - `reload` - Re-read state from the original file, discarding in-memory changes.
   - `persist` - Explicitly save state to file (when `--no-auto-persist` is enabled).
   - `exit` or `quit` - Exit the shell session.
1. By default, state is persisted after each mutation.
   - Use `--no-auto-persist` to defer writes and use explicit `persist` commands.
   - `--read-only` prevents any state modifications.
1. ❓ Shell interface improvements TBD (command history, tab completion, syntax highlighting, TUI mode with richer
   interaction).
1. ❓ Integration with spawn TBD (`spawn --shell` to immediately enter shell, possibly without requiring initial state
   file).
1. Example: `duralade entity shell --state worker-123.json`

### 4.4. Debug Operations

TODO: Debug commands (inspect, interactive, step, breakpoints, etc.)

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

## 5. History and State

### 5.1. Overview

1. Entity state is an append-only event log.
   - The current execution state is derived by replaying all events in order.
   - Each tick appends new events to the log.
   - 💭 Why event log? Provides complete history for time-travel debugging, deterministic replay, and auditability.
1. Events serialize to multiple formats, with JSON as the default.
   - Format inferred from file extension or specified via `--state-format`.
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
   - `event_type: str` - Event type identifier (e.g., "DuraladeEntityCancel").
   - `event_num: int` - Monotonically increasing event number (may have gaps).
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
     - `request_event_num: int` - References the event_num of the DuraladeExternInvoke.
     - `result: user_payload[map[str, any]]` - Extern function's out fields.
   - `DuraladeExternInvoke` - Extern invocation requested by Duralade code. **Written by Duralade.**
     - `extern: str` - Name of the extern function being invoked.
     - `args: user_payload[map[str, any]]` - Extern function's in/inout fields.
   - `DuraladePatchSelect` - Patch version selected during execution. **Written by Duralade.**
     - `version: str` - Qualified identifier of the selected patch version (e.g., "my_feature.v2").
   - `DuraladeFuncComplete` - Function invocation completed. **Written by Duralade.**
     - `request_event_num: int` - References the event_num of the DuraladeFuncInvoke.
     - `result: user_payload[map[str, any]]` - Function's out fields.
   - `DuraladeFuncInvoke` - Function invocation requested. **Written by user.**
     - `id: str?` - Optional user-provided identifier for this function invocation.
     - `func: str` - Name of the function to invoke.
     - `args: user_payload[map[str, any]]` - Function's in/inout fields.

## 6. Debugging

TODO: Stepping through execution, rewinding/time-travel debugging, breakpoints, state inspection.
