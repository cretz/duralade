# Parser Implementation Notes

Just jotting down ideas for how to implement the Duralade parser/compiler.

## `duralade-syntax` Crate

**Purpose:** Lexing, parsing, AST, type system, and semantic analysis - the complete front-end.

**Key features:**
- **Lexer + Parser**: Converts source text → AST
  - Hand-written recursive descent parser
  - Integrated lexer (no separate token stream unless needed)
  - Optimized for partial parsing (LSP/IDE support)
  - Handles incomplete/invalid code gracefully for tooling
  - Strict formatting validation (120 char lines, 4-space indent, etc.)

- **AST types**: All syntax tree node definitions
  - `#[derive(Serialize, Deserialize)]` on all nodes
  - Type information stored as `Option<TypeInfo>` fields
  - Nodes enriched with types during/after parsing
  - Can be serialized for build caching

- **Type system**: Type representation and operations
  - Types: primitives, nilable, generics, entity refs, anonymous types, etc.
  - Type equality, assignability checking
  - Constraint satisfaction for generic parameters
  - Type introspection (@type, @typein, @typeout)

- **Semantic analysis**: Name resolution and type checking
  - Two-phase approach:
    1. **File-level**: Parse all files, resolve construct signatures and field types
    2. **Function-level**: Type check function/init/run bodies (statements, expressions)
  - Lazy type resolution: parser queries context for types as needed
  - Supports circular module imports (resolve all signatures before checking bodies)
  - Cycle detection for field default expressions within constructs
  - View/noblock constraint enforcement
  - Implicit context validation

- **API flexibility**:
  - Can parse without type checking (for tooling, syntax highlighting)
  - Can parse with incremental type checking (query context lazily)
  - Can do full two-phase type checking (for compilation)

- **no_std capable**: Pure computation, no I/O dependencies

**Inputs:**
- Source text (UTF-8 strings)
- Context providing type information from other modules (for lazy resolution)

**Outputs:**
- AST with optional type annotations
- Diagnostic errors with source locations

**Dependencies:**
- Minimal: likely just serde for serialization

---

## Next Crates to Define

- Runtime / interpreter
- CLI
- Build system
- Standard library implementation
