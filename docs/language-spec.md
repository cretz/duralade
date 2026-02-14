# Duralade Language Specification

## Table of Contents

1. [Language Overview](#1-language-overview)
2. [Notation and Conventions](#2-notation-and-conventions)
3. [Source Code Representation](#3-source-code-representation)
4. [Comments](#4-comments)
5. [Identifiers](#5-identifiers)
6. [Source Files and Modules](#6-source-files-and-modules)
   - 6.1. [Source Files](#61-source-files)
   - 6.2. [Modules](#62-modules)
   - 6.3. [Imports](#63-imports)
   - 6.4. [Annotations](#64-annotations)
   - 6.5. [Builtins](#65-builtins)
7. [Types](#7-types)
   - 7.1. [Overview](#71-overview)
   - 7.2. [Generics](#72-generics)
   - 7.3. [Entity References](#73-entity-references)
   - 7.4. [Type Introspection](#74-type-introspection)
   - 7.5. [Anonymous Types](#75-anonymous-types)
8. [Constructs](#8-constructs)
   - 8.1. [Overview](#81-overview)
   - 8.2. [Fields](#82-fields)
   - 8.3. [Type Aliases](#83-type-aliases)
   - 8.4. [Data](#84-data)
   - 8.5. [Entities](#85-entities)
   - 8.6. [Functions](#86-functions)
   - 8.7. [Externs and Natives](#87-externs-and-natives)
9. [Statements](#9-statements)
   - 9.1. [Overview](#91-overview)
   - 9.2. [Blocks](#92-blocks)
   - 9.3. [Variable Declarations and Assignments](#93-variable-declarations-and-assignments)
   - 9.4. [If](#94-if)
   - 9.5. [For](#95-for)
   - 9.6. [Defer](#96-defer)
   - 9.7. [Return](#97-return)
   - 9.8. [Implicitly](#98-implicitly)
   - 9.9. [Patch](#99-patch)
10. [Expressions](#10-expressions)
    - 10.1. [Overview](#101-overview)
    - 10.2. [Outer Scope Access](#102-outer-scope-access)
    - 10.3. [Unary](#103-unary)
    - 10.4. [Binary](#104-binary)
    - 10.5. [Type Narrowing](#105-type-narrowing)
    - 10.6. [Invocation](#106-invocation)
    - 10.7. [Literals](#107-literals)
    - 10.8. [Wait](#108-wait)
    - 10.9. [Spawn](#109-spawn)
    - 10.10. [Anonymous Functions and Data](#1010-anonymous-functions-and-data)

## 1. Language Overview

1. This document is the specification for Duralade language syntax.
   - Specification for the Duralade runtime/engine is in a separate document.
   - WARNING: Duralade is not yet stable and therefore nothing here is stable.
1. Brief overview of general Duralade features:
   - Statically/strongly typed, garbage collected.
   - Durable execution programming language designed for code that can run for extended periods (days, months, years).
   - All state is serializable and resumable at any point.
   - All code is deterministic; `extern`s are the side-effecting, non-deterministic components.
   - Built-in support for checking whether code changes are compatible with previous versions.
   - Engine can be "stepped" per instruction or "ticked" to run until all execution paths yield awaiting external
     stimulus.
1. Brief overview of general Duralade language syntax features, expanded upon later in this document:
   - Code is UTF-8 encoded.
   - Constructs and statements are separated by newlines.
   - Comments are `#` prefixed.
1. Strict formatting is part of the specification itself, not a separate tool.
   - Tools may accept a looser format while typing, show format violations during editing, and automatically format on
     save.
   - This is known as "strict format".
   - 🔒 Multi-line comma-separated elements must have a trailing comma after each element, including the last.
   - 💭 Why strict formatting? Ensuring strict formatting allows predictability for tooling and humans, and aligns with
     the deterministic nature of Duralade.

## 2. Notation and Conventions

1. This specification follows the formatting guidelines described in `spec-format.md`.
1. Syntax notation at the top of relevant sections uses a variant of
   [Wirth Syntax Notation](https://en.wikipedia.org/wiki/Wirth_syntax_notation).
   - Basically the same as the Go programming language specification.
   - Defined in itself:
     ```
     syntax = { production } .
     production = production_name "=" [ expression ] "." .
     expression = term { "|" term } .
     term = factor { factor } .
     factor = production_name | token [ "…" token ] | group | option | repetition .
     group = "(" expression ")" .
     option = "[" expression "]" .
     repetition = "{" expression "}" .
     ```
   - Productions are expressions constructed from terms and the following operators, in increasing precedence:
     - `|` - Alternation.
     - `()` - Grouping.
     - `[]` - Option (0 or 1 times).
     - `{}` - Repetition (0 to n times).
   - Lexical tokens are enclosed in double quotes `""` or backticks ` `` `.
   - The form `a … b` represents the set of characters from `a` through `b` as alternatives. The horizontal ellipsis `…`
     is also used elsewhere in the spec to informally denote various enumerations or code snippets that are not further
     specified.

## 3. Source Code Representation

```wirth
newline = "\n" .
space = " " .
unicode_char = /* an arbitrary Unicode code point except newline */ .
unicode_letter = /* a Unicode code point categorized as "Letter" */ .
unicode_lowercase_letter = /* a Unicode code point categorized as "Letter, lowercase" */ .
unicode_digit = /* a Unicode code point categorized as "Number, decimal digit" */ .
```

1. Source code must be UTF-8 encoded without BOM (Byte Order Mark).
   - 💭 Why without BOM instead of BOM optional? Everything about Duralade is as deterministic as possible, including
     the source file requirements.
1. The language is case-sensitive.
   - Identifiers, keywords, and type names are all distinguished by case.
1. Newlines are represented by either `\n` (U+000A) or `\r\n` (U+000D U+000A).
   - Both Unix-style (`\n`) and Windows-style (`\r\n`) line endings are accepted.
   - Standalone carriage return `\r` not followed by `\n` is a parse error.
   - 💭 Why accept both? Cross-platform compatibility. Most modern languages accept both to work well on all systems.
1. 🔒 Source code lines should not exceed 120 characters.
   - This is a recommendation, not a hard requirement.
   - Tools should warn about violations but still accept the code.
   - Exception: lines where the content after indentation (and after `# ` for comments) contains no spaces are exempt,
     since there is no natural break point. This covers URLs, long qualified names, and similar unbreakable tokens.
1. 🔒 Only space characters (U+0020) are allowed for indentation and whitespace outside of string literals.
   - Tab characters (U+0009) are not allowed outside of string literals.
   - 💭 Why no tabs? Tabs render differently in different editors and contexts. Using only spaces ensures code looks
     identical everywhere, which is critical for deterministic formatting.
1. 🔒 Indentation must be 4 spaces per indentation level.
   - Each nested level of code adds 4 spaces to the indentation.
1. Only space (U+0020) and newlines (`\n` or `\r\n`) are valid whitespace characters.
   - Other whitespace characters (tabs, etc.) are parse errors.
   - 🔒 No trailing whitespace at end of lines or end of file.
   - 🔒 No multiple consecutive blank lines.
   - 🔒 Files must end with a newline character.

## 4. Comments

```wirth
comment = "#" space { unicode_char } .
```

1. Comments start with `#` followed by a space and continue to the end of the line.
   - The space after `#` is required.
   - The `#` character, space, and everything after until the newline is part of the comment.
   - Comments do not nest.
1. Comments must be the only content on their line.
   - 🔒 Comments must be on their own line (no trailing comments) and follow the current indentation level.
   - ❓ Should we add a separate construct for tool directives (e.g., linter overrides) since end-of-line comments are
     not allowed?
   - 💭 Why not allow trailing comments? With a 120 character line limit and the need for clear, readable code, trailing
     comments would often cause lines to exceed the limit or force awkward formatting.
1. Comments serve as documentation depending on their placement.
   - A comment immediately before a construct (entity, data, func, etc.) documents that construct.
   - A comment block at the beginning of a file (before any other content) documents the module.
   - ❓ Should we introduce a separate syntax for documentation comments to distinguish them from regular comments?

## 5. Identifiers

```wirth
identifier_normal = (unicode_lowercase_letter | "_") { unicode_lowercase_letter | unicode_digit | "_" } .
identifier_raw = "`" { unicode_char | "``" } "`" .
identifier = identifier_normal | identifier_raw .
identifier_qualified = identifier { "." identifier } .
```

1. Normal identifiers must start with a lowercase letter or underscore.
   - After the first character, lowercase letters, digits, and underscores are allowed.
   - Example: `user_name`, `calculate_total`, `x`, `_temp`, `value123`
   - 💭 Why limit to lowercase and underscores? Identifiers are not meant to support arbitrary casing. This enforces
     consistent naming across all Duralade code. In situations where name customization is required (e.g., JSON field
     names), explicit conversion functions should be used rather than relying on identifier names.
1. 🔒 Alphabetical ordering of identifiers uses ASCII order, except underscores sort after all other characters (e.g., `foo` < `foo_bar` < `_foo`).
1. Raw identifiers are enclosed in backticks.
   - The backticks are delimiters, not part of the identifier itself.
   - Raw identifiers can contain any Unicode characters, including backticks, newlines, and non-printable characters.
   - Raw identifiers are primarily for interoperability and code generation scenarios.
   - Example: `` `entity` ``, `` `User Name` ``, `` `value-with-dashes` ``
   - Escaping mechanism: Two consecutive backticks ` ` `` represent a single literal backtick within the identifier.
   - Note: Single backticks delimit raw identifiers, while two or more backticks delimit raw strings (Section 10.7).
1. There are no global reserved keywords in Duralade.
   - Keywords are context-specific and determined by the grammar.
   - What is a keyword in one context (e.g., `entity` where a construct is expected) may be a valid identifier in
     another context (e.g., as a variable name).
1. Import paths consist of identifiers separated by `.`.
   - Used in import declarations to specify module paths.
   - Example: `import my.long.module`

## 6. Source Files and Modules

### 6.1. Source Files

```wirth
source_file = { annotation } { import } { file_construct } .
```

1. Source files have the `.dl` file extension.
1. Source files must be UTF-8 encoded without BOM (as specified in Section 3).
1. A source file consists of optional module-level annotations, import declarations, and file-level constructs.
   - Module-level annotations appear first and attach to the module.
   - Import declarations come after module-level annotations.
   - File-level constructs come after all imports.
   - 🔒 There must be a blank line after module-level annotations (before imports or first construct).
   - 🔒 There must be a blank line between the last import declaration and the first file construct.
   - 🔒 There must be a blank line between each file construct.

### 6.2. Modules

```wirth
module_access = identifier "::" identifier_qualified .
```

1. Each source file defines or contributes to a module.
   - The module name is derived from the filename: the portion before the first dot (excluding the `.dl` extension).
   - Multiple files can contribute to the same module by sharing the same prefix.
   - Example: `user.dl`, `user.admin.dl`, and `user.types.dl` all contribute to the `user` module.
   - Further details on module organization and resolution are defined in the runtime specification.
1. Items from an imported module are accessed using `::` (the module access operator).
   - Example: `error::fail(msg)`, `time::sleep(duration)`, `:time::duration?`
   - This applies in both type position and expression position.
   - `.` (dot) is used exclusively for field and method access on values.
   - 💭 Why `::` instead of `.`? Using separate operators for module access (`::`) and value access (`.`) eliminates
     ambiguity when a local variable shadows a module alias (e.g., `out! :error?` creating a local `error` that shadows
     the `error` module).
1. A construct with the same name as the module can be accessed using just the module name when imported (shorthand
   form).
   - Example: If module `user` contains `out data user { ... }`, then importing `user` allows accessing the data type as
     just `user` instead of `user::user`.
   - This applies to `entity`, `data`, or `func` constructs.
   - ❓ Should the shorthand form also apply to type aliases?

### 6.3. Imports

```wirth
import_alias = "as" identifier .
import = "import" identifier_qualified [ import_alias ] .
```

1. Imports begin with the `import` keyword followed by a dot-separated module path.
1. Imports can be aliased with the `as` keyword.
   - Example: `import my.long.module as m`
   - The alias becomes the name used to access items from that module.
1. Without an alias, the rightmost identifier in the path is used as the module alias.
   - Example: `import my.long.module` makes items accessible as `module::thing`
1. All items from an imported module must be accessed with `::` qualification (Go-style).
   - Example: `module::user`, `m::process`
   - Exception: The shorthand form described in Section 6.2 allows unqualified access for module-named constructs.
1. 🔒 Import declarations must be in alphabetical order by the module path (the qualified identifier after `import`,
   before any `as` clause).
   - Example: `import a.z as x` comes before `import b.a as y` (sorted by `a.z` vs `b.a`, not by alias).

### 6.4. Annotations

```wirth
annotation = "@" invocation .
```

1. Annotations attach data instances to modules, constructs, or fields using `@` followed by a data construction.
   - File-level annotations appear before imports and attach to the module.
   - Construct-level annotations appear immediately before the construct.
   - Field-level annotations appear immediately before the field.
   - Example: `@doc(summary = "User entity documentation")`
   - 🔒 No blank lines are allowed between annotations and their target.
   - 💭 Why data instances? Makes annotations type-safe, extensible, and reuses existing syntax.
   - ❓ Primary field concept TBD - would allow `@doc("text")` instead of `@doc(summary = "text")`.
1. Multiple annotations can be attached to the same target, each on its own line.
1. Annotation arguments must be compile-time constant.
   - Only literals and calls to top-level `view` functions are allowed.

### 6.5. Builtins

1. The standard library uses the `builtin` keyword on `data`, `entity`, and their member functions for runtime-provided
   implementations. Requires `allow_builtin = true` in `duralade.toml`. Not available to user code.

## 7. Types

### 7.1. Overview

```wirth
type_argument = identifier [ "=" type ] .
type_arguments = "[" type_argument { "," type_argument } [ "," ] "]" .
type_named = ( module_access | identifier_qualified ) [ type_arguments ] .
type = (type_named | type_entity_ref | type_introspection | type_anonymous) [ "?" ] .
```

1. Types are referenced by identifier.
   - Simple: `int`, `user`
   - Qualified: `my_module::user`
   - Member access: `my_module::my_entity.my_func` (e.g., for `@typeout`)
1. Types can be nilable by appending `?`.
   - Non-nilable types cannot hold `nil`.
   - Nilable types (with `?`) can hold `nil` or a value of the base type.
   - Example: `int` cannot be nil, but `int?` can be nil or an integer value.
1. Built-in types.
   - `int` - Unbounded signed integer type.
   - `float` - 64-bit IEEE 754 floating-point type (without NaN or Infinity).
   - `str` - UTF-8 string type.
   - `bool` - Boolean type.
   - `any` - Top serializable type. All non-nilable serializable types are assignable to `any`.
   - `anylocal` - Top type including non-serializable types (`func`, `entity`). All non-nilable types are assignable
     to `anylocal`.
   - Additional semantics and representation details are defined in the runtime specification.
   - 💭 Why separate `any` and `anylocal`? `any` is the safe default for most code. `anylocal` explicitly opts in to
     non-serializable values that cannot cross serialization boundaries.
   - ❓ Should Duralade support union types (e.g., `int | str | bool`)?
1. Serializability.
   - Serializable types: `int`, `float`, `str`, `bool`, `nil`, entity references (`&`), and data whose fields are all
     transitively serializable.
   - Non-serializable types: `func`, `entity`.
   - Generic data serializability depends on type arguments: `array[t = int]` is serializable, `array[t = func]` is not.
   - The type checker enforces serializability at serialization boundaries (see Sections 7.3, 8.7).
1. Inferred types must be concrete.
   - `nil`, `[]`, and `{ }` require explicit type annotations (`int?`, `array[t = int]`, `map[k = str, v = int]`).
   - Collection literals with mixed element types (e.g. `[1, "hello"]`) are errors - use `array[t = any]` explicitly.
   - Uniform literals infer normally: `[1, 2, 3]` infers as `array[t = int]`.

### 7.2. Generics

1. Generic types are parameterized with type arguments in square brackets.
   - 🔒 Spaces are required around `=` in type arguments.
   - Example: `list[t = int]`, `map[key = str, value = int]`
   - 💭 Why `=` instead of `:`? See Section 10.6 for rationale - consistency in using `=` for binding values/types.
1. Type arguments support shorthand syntax (see Section 10.6 for invocation shorthand rules).
   - Example: `list[t]` is shorthand for `list[t = t]`.
   - 🔒 When an identifier or access expression matches the type parameter name, the shorthand form must be used.
1. Type parameters are declared using `intype` (see Section 8.2).
   - Supported in entities, data, functions, externs, and natives.
1. Type parameter constraints control which types are accepted (see Section 8.2 for full rules).
   - `intype t` - constrained to serializable types (`any`).
   - `intype t: anylocal` - accepts any type including non-serializable (`func`, `entity`).
1. Type arguments may be omitted when the type parameter has a default expression (see Section 8.2).
   - When omitted, the default expression is evaluated to determine the type argument.
1. Type arguments without defaults are inferred from argument types when omitted.
   - Unresolved type parameters after inference are errors.
1. Explicit type arguments must be assignable to the type parameter's constraint.
   - ❓ Variance rules TBD.

### 7.3. Entity References

```wirth
type_entity_ref = "&" type .
```

1. Entity reference types are denoted with the `&` prefix.
   - Example: `&my_entity`, `&list[t = int]`
1. Entity references refer to spawned or detached entities.
   - Entity references are returned by the `spawn` builtin (see runtime specification).
1. Entity reference fields and methods are accessed using the `->` operator (see Section 10.1).
   - The `->` operator performs blocking calls to the referenced entity.
   - All arguments and results of `->` calls must be serializable types (see Section 7.1).
   - Example: `entity_ref->method()` blocks while calling the method, `entity_ref->field` blocks while reading the
     field.
   - 💭 Why a different operator? The `->` operator makes blocking/remote calls immediately visible in the code.
1. Entity references cannot be used to make calls within `view` or `noblock` functions.
   - Entity references can be passed as parameters within `view`/`noblock` functions.
   - ❓ Special operations on entity references (e.g., `.id`, `.cancel()`, `.terminate()`) TBD.
   - ❓ Operator for accessing all `out` fields at once (e.g., `->*` or similar) TBD.

### 7.4. Type Introspection

```wirth
type_introspection = type_named ("@type" | "@typein" | "@typeout") .
```

1. Type introspection operators extract type information from identifiers.
   - Type introspection operators are used in type positions.
   - Type introspection operators evaluate to the builtin `type` type or anonymous data types.
1. The `@type` operator gets the type of an identifier.
   - Valid on fields, variables, and parameters.
   - Example: `item@type` returns the type of the `item` field.
1. The `@typein` operator extracts input types from entities or functions.
   - Returns an anonymous data type containing all `in` and `inout` fields.
   - Example: `my_func@typein` returns an anonymous type with the function's input fields.
1. The `@typeout` operator extracts output types from entities or functions.
   - Returns an anonymous data type containing all `out` and `inout` fields, and the `out!` field if present.
   - The returned anonymous type can include an `out!` field, which regular data types cannot define.
   - Invoking an entity or function returns an instance of its `@typeout` type.
   - Example: `my_func@typeout` returns an anonymous type with the function's output fields.
   - ❓ Notation for referencing an entity's `run` function's out type TBD. Current candidate: `e.run@typeout` where `e`
     is an entity value. Needed for stdlib `entity::run_result` and similar operations.
1. Type introspection can be used at runtime for type comparisons.
   - Types are first-class values of the builtin `type` type.
   - Example: `if x := value as some_value@type {`
   - ❓ Runtime operations on type values (beyond equality comparison) TBD.
1. ❓ Assignability between anonymous types from type introspection and user-defined data types TBD.

### 7.5. Anonymous Types

```wirth
type_anonymous_field_modifier = "in" | "out" | "out!" | "inout" .
type_anonymous_field = [ type_anonymous_field_modifier ] [ identifier ] ":" type [ "=" "???" ] .
type_anonymous = ("data" | ([ "view" | "noblock" ] "func") | "entity") "{" { type_anonymous_field } "}" .
```

1. Anonymous types are structural types defined inline without a name.
   - Anonymous data types: `data { field: int }`
   - Anonymous function types: `func { in x: int }`, `view func { in x: int }`
   - Anonymous entity types: `entity { in x: int }`
   - Fields are newline-separated (same rules as construct bodies).
1. Anonymous types are structurally typed (duck-typed).
   - Any type with matching fields satisfies the anonymous type.
   - Field order does not matter for structural compatibility.
1. Named types can be assigned to anonymous types if structurally compatible.
   - Anonymous types cannot be assigned to named types.
   - Example: `var x: data { field: int } = my_named_data` works if `my_named_data` has a compatible `field`.
1. Optional fields in anonymous types use `= ???` to indicate a default exists without specifying its value.
   - Applies to `in` and `inout` fields in anonymous function and entity types.
   - `out` fields always have implicit defaults and do not need `= ???`.
   - Example: `func { in required: int, in optional: str = ??? }` indicates `optional` has a default.
1. Anonymous types can include `out!` fields, including in anonymous data types.
   - Anonymous data with `out!` fields cannot be defined as named data types.
   - This is primarily used for types returned by `@typeout` (see Section 7.4).
   - Example: `data { result: str, out! :error? }` is a valid anonymous data type.
1. Anonymous types can be used anywhere a type can be used.
   - Type annotations, function parameters, return types, constraints, etc.
   - Anonymous type literals and expressions are specified in Section 10.10.

## 8. Constructs

### 8.1. Overview

```wirth
file_construct = { annotation } (type | data | entity | func | extern | native) .
```

1. Duralade has six file-level construct types:
   - `type` - Type alias definitions (detailed in Section 8.3).
   - `data` - Data structure definitions (detailed in Section 8.4).
   - `entity` - Object definitions (detailed in Section 8.5).
   - `func` - Function definitions (detailed in Section 8.6).
   - `extern` - External function declarations (detailed in Section 8.7).
   - `native` - Native function declarations (detailed in Section 8.7).
1. Constructs can be marked with the `out` visibility modifier.
   - `out` constructs are visible and accessible outside the module.
   - The `out` modifier applies to `type`, `data`, `entity`, and `func` constructs.
   - `extern` and `native` constructs cannot be marked `out` and are never accessible outside their module.
1. 🔒 File-level constructs must appear in the following order:
   - `type` constructs, first `out` then non-`out`, alphabetically by name within each group.
   - `data` constructs, first `out` then non-`out`, alphabetically by name within each group.
   - `entity` constructs, first `out` then non-`out`, alphabetically by name within each group.
   - `func` constructs, first `out` then non-`out`, alphabetically by name within each group.
   - `extern` constructs, alphabetically by name.
   - `native` constructs, alphabetically by name.
   - 💭 Why strict ordering? Ensures deterministic, predictable file organization and makes constructs easy to locate.

### 8.2. Fields

```wirth
field_intype = "intype" identifier [ ":" type ] [ "=" type ] .

field_signature = (identifier ":" type [ "=" expression ]) |
                  (":" type [ "=" expression ]) |
                  (identifier "=" expression) .

field_var = ("in" | "out" | "out!" | "inout" | "value" | "implicit") field_signature .

field = { annotation } (field_intype | field_var) .
```

1. Fields in all constructs except `data` must begin with a modifier: `intype`, `in`, `out`, `out!`, `inout`, `value`,
   or `implicit`.
   - `data` construct fields must not have modifiers (see Section 8.4).
   - Which modifiers are allowed depends on the construct (see specific construct sections).
   - `value` fields are only allowed in `entity` constructs (see Section 8.5).
   - `implicit` fields are only allowed in functions (including `init`, `run`, and member functions).
   - 💭 Why no modifier for externally-mutable entity fields? External mutation is only useful when an entity is waiting
     on a field. When needed, explicit `noblock` setter functions are clear and flexible.
1. At most one field can be marked `out!` (early-return field).
   - The `out!` field is used with early-return statements and the `!` operator.
   - `out!` fields must be nilable types.
   - `out!` fields cannot have default expressions.
1. Type parameter fields (`intype`) consist of an identifier, optional constraint, and optional default type.
   - Type parameters without constraints are implicitly constrained to `any?` (serializable types only).
   - The constraint `anylocal` allows non-serializable types (`func`, `entity`) as type arguments.
   - Type parameters without defaults are inferred from arguments or provided explicitly (see Section 7.2).
   - When a default expression is present without an explicit constraint, the constraint is implicitly the same as the
     default.
   - Example: `intype t` - requires explicit type argument, constraint is `any?` (serializable).
   - Example: `intype t: anylocal` - accepts any type including non-serializable.
   - Example: `intype t = int` - defaults to `int`, explicit arguments must be assignable to `int`.
   - Example: `intype t: anylocal = int` - defaults to `int`, accepts any type.
   - Example: `intype t = item@type` - derives type from `item` field, explicit arguments must be assignable to
     `item@type`.
   - 💭 Why implicit constraint from default? This ensures that explicit type arguments are assignable to the default
     type, maintaining type safety.
   - ❓ Additional constraint types and variance are TBD.
1. Other fields (`in`, `out`, `out!`, `inout`, `value`, `implicit`) require a field signature.
1. Field signatures can omit the identifier if it matches the type name.
   - Example: `in :user` is shorthand for `in user: user`
   - 🔒 When the identifier matches the type and no expression is present, the shorthand form must be used.
   - 🔒 When the identifier matches the type and an expression is present, use `:type = expression` form.
1. Field signatures can omit the type if a default expression is provided.
   - The type is inferred from the expression.
   - Example: `in count = 0` infers `count` as `int`
   - 🔒 When an expression is provided and the inferred type matches the expression type, the type annotation must be
     omitted.
1. Fields with neither a type annotation nor a default expression are invalid.
1. Default expression requirements depend on field modifier:
   - `in` and `inout`: No default means required parameter. Nilable fields must have explicit `= nil` for optional.
   - `out`, `value`, and `implicit`: Must have defaults. Nilable types implicitly default to `= nil`; non-nilable types
     require explicit defaults.
   - `out!`: Cannot have default expressions (implicitly defaults to nil).
1. Implicit fields are unique by type across the implicit context stack.
   - Only one implicit value per type can exist at any point in execution.
   - Defining an `implicit` field shadows any outer implicit of the same type.
   - Field names are for local reference; types are used for lookup.
   - Implicit field types must be concrete named types. Anonymous types, duck types, and `any` are not allowed.
   - Lookup matches by the base (non-nilable) named type against the implicit context stack (see Section 9.8).
   - `implicit :foo?` (nilable): if no matching implicit is in the context, the field receives nil. If a matching
     implicit is found with a nil value, the field receives nil.
   - `implicit :foo` (non-nilable): if no matching implicit is in the context and no default is provided, this is a
     runtime error. If a matching implicit is found but the value is nil, this is also a runtime error.
1. Field default expressions must not have side effects.
   - Allowed: literals, references to other fields, view function calls, data/entity construction.
   - Not allowed: non-view function calls, extern calls, statements.
   - Field default expressions can reference other fields, including fields defined later in the source.
   - Circular references between field default expressions are a compiler error.
   - 💭 Why? This ensures field initialization is pure computation with no observable evaluation order, allowing the
     compiler to evaluate fields in dependency order regardless of definition order.
1. 🔒 Fields must appear in the following order:
   - `intype` fields, alphabetically by name.
   - `in` fields without default expressions, alphabetically by name.
   - `in` fields with default expressions, alphabetically by name.
   - `inout` fields without default expressions, alphabetically by name.
   - `inout` fields with default expressions, alphabetically by name.
   - `out` fields, alphabetically by name.
   - `out!` field (if present).
   - `implicit` fields, alphabetically by name.
   - `value` fields, alphabetically by name.
   - ❓ Primary `in` field TBD - would allow `doc("text")` instead of `doc(summary = "text")`.
1. When no more meaningful name exists, use `value` for the primary `in`/`inout` field and `result` for the primary `out` field. In practice, `result` is more commonly needed since input fields tend to have naturally descriptive names.

### 8.3. Type Aliases

```wirth
type_alias = [ "out" ] "type" identifier [ "{" { field_intype } "}" ] "=" type .
```

1. Type aliases create alternative names for existing types.
   - Example: `type user_id = int`
1. The `out` modifier (see Section 8.1) makes type aliases visible outside the module.
1. Type aliases can have type parameters using `intype` fields.
   - Type parameters are declared in braces before the `=`.
   - Example: `type result { intype t } = data { value: t, out! :error? }`
1. Type aliases are transparent (fully interchangeable with their target type).
   - Assigning between an alias and its target type requires no conversion.

### 8.4. Data

```wirth
data_modifier = "out" .
data_body = { field_signature } { func } .
data = [ data_modifier ] "data" identifier "{" data_body "}" .
```

1. Data represents value-type structures.
1. Data is declared with the `data` keyword followed by a name and body.
1. The `out` modifier (see Section 8.1) makes data visible outside the module.
1. Data fields do not use field modifiers.
   - All fields are implicitly `inout` (both input parameters and accessible outputs).
   - Fields use the `field_signature` syntax from Section 8.2 without a modifier keyword.
1. Default expression rules for data fields follow the `inout` rules from Section 8.2.
   - No default means required parameter.
   - Nilable fields must have explicit `= nil` for optional parameters.
1. Data serializability is transitive (see Section 7.1).
   - A data type is serializable when all its fields are transitively serializable.
   - Data with non-serializable fields (e.g., `func`, `entity`, or generic data parameterized with non-serializable
     types) is itself non-serializable and cannot be used at serialization boundaries.
   - 🔒 Data fields must be in alphabetical order by name.
1. Data can have member functions (see Section 8.6).
   - All member functions in data are implicitly `noblock`.
   - Member functions can have `out` visibility and `view` runtime modifiers (but not explicit `noblock` since it's
     implied).
   - Member functions can access the data's fields directly.
   - 💭 Why these restrictions? Data is designed for value-type structures and simple helpers (field manipulation,
     computed properties, maintaining invariants). Complex behavior, long-running operations, or side effects belong in
     entities or free functions.
   - ❓ Equality and hashing semantics for data instances are TBD.

### 8.5. Entities

```wirth
entity_modifier = "out" .
entity_init_func = "init" "{" { field } { statement } "}" .
entity_run_func = "run" "{" { field } { statement } "}" .
entity_body = { field } [ entity_init_func ] [ entity_run_func ] { func } .
entity = [ entity_modifier ] "entity" identifier "{" entity_body "}" .
```

1. Entities are objects that can be suspended and resumed.
   - Entities are suitable for both short-lived operations and long-running processes.
1. Entities are declared with the `entity` keyword followed by a name and body.
1. The `out` modifier (see Section 8.1) makes entities visible outside the module.
1. Entities support the following field modifiers:
   - `intype` - Type parameters for the entity.
   - `in` - Input parameters provided when the entity is constructed.
   - `out` - Output fields that can be accessed at any time during entity execution, even before the entity completes.
   - `inout` - Combination of `in` and `out` behavior.
   - `value` - Internal state that is part of the entity's serialized state. Not accessible from outside the entity.
1. Entity field modifiers have specific semantics:
   - `in` fields are immutable after construction.
   - `out` and `inout` fields can be mutated by the entity.
   - `value` fields are only accessible to member functions of the entity.
1. Entities can have an `init` function.
   - The `init` function executes during entity construction.
   - The `init` function is implicitly `noblock` (non-blocking).
   - The `init` function can only have `implicit` fields (see Section 8.2).
1. Entities can have a `run` function.
   - The `run` function executes in the background after entity construction.
   - The `run` function can have `out`, `out!`, and `implicit` fields (see Section 8.2).
   - The `out` fields represent the entity's return values when it completes normally.
   - The `out!` field is used for early completion (commonly `out! :error?` for error handling).
   - The entity completes when the `run` function returns.
   - If no `run` function is present, the entity runs indefinitely until explicitly terminated.
   - ❓ How can callers cancel or terminate an entity (especially those without `run` or with infinite execution)? TBD.
1. Entities can have member functions (see Section 8.6).
   - Member functions can have `out` visibility and `view`/`noblock` runtime modifiers.
   - Member functions can access the entity's fields directly.
1. Entities can be constructed locally via normal invocation (like data construction).
   - Entities with `run` cannot be constructed locally - they must use `spawn`.
1. Entities can be created via `spawn` (see Section 10.9), which returns an entity reference.
   - ❓ Whether entities with `run` should ever support local construction TBD.
1. Entity lifecycle and usage semantics have open design questions.
   - ❓ Cancellation and termination semantics for spawned entities.
   - ❓ Garbage collection behavior for entities with background `run` coroutines.
   - ❓ Remote execution semantics - see Section 10.9.

### 8.6. Functions

```wirth
func = [ "out" ] [ "view" | "noblock" ] "func" identifier "{" func_body "}" .
func_body = { field } { statement } .
```

1. Functions are declared with the `func` keyword followed by a name and body.
1. The `out` modifier (see Section 8.1) makes functions visible outside the module.
1. Functions can have at most one runtime modifier: `view` or `noblock`.
   - `view` indicates read-only, non-blocking behavior and implies `noblock`.
   - `noblock` guarantees no blocking calls but allows mutation.
   - 💭 Why separate modifiers? `noblock` functions can mutate local state, while `view` functions provide stronger
     guarantees about read-only behavior.
1. `view` functions have calling restrictions.
   - On parameters or external values: can only call `view` functions.
   - On locally-created values: can call any `noblock` function.
   - 💭 Why? Parameters belong to the caller and must not be mutated. Locally-created values are owned by the function.
1. `noblock` functions can only call other `noblock` or `view` functions.
1. `view` functions cannot construct entities with `run` functions (starting coroutines is a mutation).
1. `noblock` functions can construct entities with or without `run` functions (starting coroutines doesn't block).
1. Functions support field modifiers `intype`, `in`, `out`, `out!`, and `implicit` (see Section 8.2).

### 8.7. Externs and Natives

```wirth
extern = "extern" identifier "{" { field } "}" .
native = "native" ( "view" | "noblock" ) identifier "{" { field } "}" .
```

1. Externs and native functions are external function declarations implemented outside Duralade.
1. `extern` declares blocking, side-effecting, non-deterministic operations.
   - Results are memoized by the runtime for deterministic replay.
   - All `in`, `inout`, `out`, and `out!` fields must be serializable types (see Section 7.1).
   - Cannot be called from `view` or `noblock` contexts.
   - Common use cases: HTTP requests, database operations, file I/O, network calls, external API interactions.
1. `native` declares deterministic implementations provided outside Duralade.
   - Must have `view` or `noblock` modifier.
   - Not memoized; safe to call repeatedly during replay.
   - Common use cases: telemetry (logging, metrics, error capture), performance-critical code, platform-specific
     operations.
   - 💭 Why dangerous? The language cannot verify native function behavior. Non-deterministic behavior or constraint
     violations break determinism.
1. `extern` and `native` support field modifiers `intype`, `in`, `inout`, `out`, and `out!` (see Section 8.2).
   - `implicit` and `value` fields are not allowed.
1. For `extern` and `native`, `out` fields must not declare default expressions.
   - Output values are produced by the host implementation, not by declaration defaults.
   - Non-nilable `out` fields must be explicitly set by the host result.
   - 💭 Why? This avoids ambiguity about whether an output came from host behavior or a declaration fallback, which keeps
     replay and diagnostics deterministic and easier to reason about.
1. Native host invocations may use an asynchronous host API.
1. Host-side native failures map to the runtime error model:
   - Environment/data mismatch or handler-level failures are faults.
   - Runtime/engine bugs are internal errors.
1. Neither `extern` nor `native` can be marked `out`; both are always module-private.
1. Implementation mechanisms are defined in the runtime specification.

## 9. Statements

### 9.1. Overview

```wirth
statement = block |
            var |
            var_assignment |
            assignment |
            if |
            for |
            for_break |
            for_continue |
            defer |
            return |
            return_early |
            implicitly |
            patch |
            expression .
```

1. Statements are the executable components of function bodies.
1. Statements are separated by newlines.
   - A statement ends at a newline if the last token can legally end a statement.
   - This means expressions that continue on the next line must have the continuation token (like `.` for method
     chaining) before the newline.
   - 💭 Why? This provides clear statement boundaries without semicolons, while avoiding ambiguity in multi-line
     expressions.
1. Not all expressions are valid as statements.
   - Parenthesized expressions cannot be standalone statements.
   - Prefix unary expressions cannot be standalone statements.
   - 💭 Why these restrictions? They eliminate ambiguity with the newline-based statement termination rule. Without
     them, `foo\n(bar)` could be ambiguous (function call or two statements?), and `a\n+ b` could be unclear (binary
     operator or unary?).
   - ❓ Panic/recover statements for unrecoverable errors and error recovery TBD.

### 9.2. Blocks

```wirth
block = "{" { statement } "}" .
```

1. Blocks are sequences of statements enclosed in curly braces.
1. Blocks are used in control flow statements (`if`, `for`, `defer`) and as function bodies.
1. Blocks create a new scope for variable declarations.
   - Variables declared within a block are not accessible outside it.
1. 🔒 The opening `{` must be on the same line as the statement that introduces the block.
1. 🔒 Each statement within a block must be indented 4 spaces from the block's opening brace level.
1. 🔒 The closing `}` must be at the same indentation level as the statement that introduced the block.

### 9.3. Variable Declarations and Assignments

```wirth
var = "var" field_signature { "," field_signature } [ "=" expression { "," expression } ] .
var_assignment = identifier { "," identifier } ":=" expression { "," expression } .
assignment_op = "=" | "+=" | "-=" | "*=" | "/=" | "%=" .
assignment = expression { "," expression } assignment_op expression { "," expression } .
```

1. Variable declarations introduce new mutable variables using `var` or `:=`.
1. The `var` keyword uses `field_signature` syntax from Section 8.2.
   - Example: `var count: int = 0`
   - Example: `var count = 0` (type inferred)
   - Example: `var :int?` (shorthand for `var int: int?`, nilable, implicitly defaults to `nil`)
   - Example: `var :array[t = int] = []` (shorthand for `var array: array[t = int] = []`)
   - Example: `var user: user?` (nilable, implicitly defaults to `nil`)
   - Example: `var a, b = x, y` or `var a: int, b: str = x, y`
1. The `:=` operator declares and initializes a variable with type inference.
   - Example: `count := 0` (equivalent to `var count = 0`)
   - Example: `a, b := x, y`
1. Multi-variable forms evaluate all right-hand expressions first, then perform assignments left-to-right.
   - This allows swapping: `a, b = b, a`
   - The number of targets must match the number of expressions.
   - The identifier `_` can be used to discard values.
1. Non-nilable variables must have an explicit initializer.
   - Nilable variables implicitly default to `= nil` if no initializer is provided.
1. Variables are scoped to the block in which they are declared.
1. A variable cannot be declared with the same name as another variable in the same block.
   - Variables in inner blocks can shadow variables from outer blocks.
1. Assignment operators update the value of a mutable location.
   - `=` assigns a new value (supports multi-variable form).
   - `+=`, `-=`, `*=`, `/=`, `%=` are compound assignment operators (single-variable only).
   - ❓ Additional compound assignment operators (bitwise, logical, etc.) TBD.
1. The left-hand side of an assignment must be a mutable location.
   - Variables declared with `var` or `:=`.
   - `out` or `inout` fields.
   - Field access expressions (e.g., `obj.field`).
   - ❓ Array/map elements (syntax TBD).
1. The right-hand side is evaluated and assigned to the left-hand side.
1. Types must be compatible (right-hand side assignable to left-hand side type).

### 9.4. If

```wirth
if_condition_bool = [ (var_assignment | assignment) ";" ] expression .
if_narrowing_as = identifier ":=" expression "as" type { "," identifier ":=" expression "as" type } .
if_narrowing_nil = identifier ":=" expression "?" { "," identifier ":=" expression "?" } .
if = "if" (if_condition_bool | if_narrowing_as | if_narrowing_nil)
     block { "else" if | "else" block } .
```

1. If statements have three mutually exclusive forms.
1. Form 1: Optional init followed by boolean condition.
   - The condition expression must evaluate to type `bool`.
   - Variables declared in the init are scoped to the if block and all else clauses.
   - The init supports multi-variable declarations and assignments.
   - Example: `if x := compute(); x > 0 { ... }`
   - Example: `if a, b := x, y; a > b { ... }`
1. Form 2: Type narrowing with `as`.
   - Performs a runtime assignability check: can the value be assigned to the specified type?
   - Multi-variable form requires all type checks to succeed (AND semantics) for the if block to execute.
   - If all checks succeed, variables are bound with narrowed types and accessible only in the if block.
   - If any check fails, the else block executes and variables do not exist.
   - Works with any types, including structural compatibility checks and generic type narrowing.
   - The identifier `_` can be used to discard values.
   - Example: `if x := value as int { ... }`
   - Example: `if a, b := x as int, y as str { ... }`
   - 🔒 When unwrapping nilable types, the `?` form (Form 3) should be used instead.
   - 💭 Why scoped only to if block? The variable is conditionally bound only when the type check succeeds. In the else
     block, the check failed, so no binding occurred.
1. Form 3: Nil unwrapping with `?`.
   - Syntactic sugar for `as` with the non-nilable type.
   - Multi-variable form requires all values to be non-nil (AND semantics) for the if block to execute.
   - If all values are non-nil, the unwrapped values are bound with non-nilable types in the if block only.
   - Shadowing is encouraged to make it clear you're using the unwrapped version.
   - The identifier `_` can be used to discard values.
   - Example: `if x := x? { ... }`
   - Example: `if a, b := x?, y? { ... }`
1. Each `else if` clause can independently use any of the three forms.
   - Each `else if` is syntactic sugar for `else { if ... }` and creates a nested implicit block.
1. Original outer scope variables remain accessible in all branches (unless shadowed).
1. ❓ Combining narrowing with an additional boolean condition in one statement is TBD. Currently requires nesting.
1. ❓ Mixing narrowing forms (e.g., `if a := x?, b := y as str {`) is TBD.

### 9.5. For

```wirth
for_clause = expression | identifier "in" expression .
for = "for" [ ":" identifier ] [ for_clause ] block .
for_break = "break" [ ":" identifier ] .
for_continue = "continue" [ ":" identifier ] .
```

1. For statements execute code repeatedly.
1. For statements have three forms:
   - Infinite loop: `for { ... }`
   - Condition loop: `for condition { ... }`
   - For-in loop: `for item in expr { ... }`
1. For-in loops use the iter func pattern defined in `duralade.iter`.
   - The loop variable is re-declared for each iteration.
   - The expression after `in` must evaluate to a func (the iter func) or a compound with an `iter` field containing one.
   - The runtime constructs a `duralade.iter::yielder` entity and passes it to the iter func as a regular parameter.
   - The iter func calls `yielder.yield(value = v)` to produce values; each call executes the for body.
   - The runtime sets `yielder.active` to `false` on `break`. The iter func should check this after each `yield`.
   - `yielder.yield()` also returns `{ active: bool }` as a convenience.
1. `break` and `continue` propagate across for-in boundaries, including labeled forms targeting outer loops.
1. For statements can have an optional label for use with `break` and `continue`.
   - Example: `for:outer item in iter::range(end = 10) { ... }`
   - 🔒 Loop labels must not have spaces around the colon.
1. `break` exits the innermost loop, or the labeled loop if a label is specified.
   - Example: `break:outer`
   - 🔒 Break labels must not have spaces around the colon.
1. `continue` skips to the next iteration of the innermost loop, or the labeled loop if a label is specified.
   - 🔒 Continue labels must not have spaces around the colon.

### 9.6. Defer

```wirth
defer = "defer" block .
```

1. Defer statements schedule a block of code to execute when the function returns.
   - ❓ How can users control cancellation contexts in defer blocks (e.g., to ensure cleanup runs even in cancelled
     contexts)?
1. Multiple defer statements execute in LIFO order (last deferred, first executed).
1. Defer blocks can access variables from the outer scope that were declared before the defer statement.
   - ❓ Variable capture semantics TBD. How are variable values observed in defer blocks (at defer registration vs at
     execution)? Should defer accept function calls like Go (capturing arguments at registration)?
1. Defer blocks cannot contain early return statements (`return!` or the `!` operator).
   - 💭 Why? Early returns in cleanup code would be confusing and error-prone.
   - ❓ Should a `defer!` form exist to explicitly allow early returns?

### 9.7. Return

```wirth
return = "return" .
return_early = "return!" expression .
```

1. Return statements exit the current function and return control to the caller.
1. Normal return (`return`) returns with `out` fields as set.
1. Early return (`return!`) sets the `out!` field to the expression and returns.
   - Only valid in functions or contexts that declare an `out!` field.
   - The `!` operator on expressions can also trigger early returns (see Section 10.3).

### 9.8. Implicitly

```wirth
implicitly_block = "implicitly" expression { "," expression } [ "," ] block .
implicitly_statement = "implicitly" expression { "," expression } [ "," ] .
implicitly = implicitly_block | implicitly_statement .
```

1. Sets implicit context values available by type to called functions (see Section 8.2).
1. Two forms: block-scoped (with block) and rest-of-outer-block (without block).
1. Multiple expressions can be comma-separated. Duplicate types in one statement is a compile error.
1. Keyed by base (non-nilable) named type. Nilable expressions (`foo?`) store under the base type.
   - Must be concrete named types - no anonymous types, duck types, or `any`.
   - Use `as` to narrow broad types: `implicitly value as specific_type`.
   - Same-type values in outer scope are shadowed.
1. Does not rebind already-bound implicit fields in the current scope - only affects transitive calls.
   - To update the current binding: reassign the variable, then `implicitly` to propagate.
   - ❓ Nilable expression flowing into non-nilable `implicit` field causes runtime error. Type checker should warn. TBD.
   - ❓ Callable implicits (function signature types) TBD - requires structural/duck type matching.

### 9.9. Patch

```wirth
patch_version = identifier_qualified | "@default" .
patch_first_branch = "%" patch_version "{{" { statement } .
patch_next_branch = "%" "}}" patch_version "{{" { statement } .
patch_chain = patch_first_branch { patch_next_branch } "%" "}}" .
patch_complete = "%" identifier_qualified "complete" .
patch = patch_chain | patch_complete .
```

1. Patch statements allow different code versions during entity execution.
   - Control which statements execute without introducing variable scoping.
1. 🔒 Patch directives must start at column 0.
   - 🔒 Spaces required: `% version {{` not `%version{{`.
   - 💭 Why column 0? The `{{` delimiter would be confusing if indented since `{` creates variable scopes.
1. Version identifiers are qualified identifiers or `@default`.
   - Example: `% my_feature.v2 {{`, `% auth.change.v3 {{`, `% @default {{`
   - 🔒 Use qualified names to avoid accidental coupling.
1. Patch blocks chain like if/else, ending with `% }}`.
   - Example:
     ```
     % my_feature.v3 {{
         newest_code()
     % }} my_feature.v2 {{
         older_code()
     % }} @default {{
         original_code()
     % }}
     ```
1. Version selection is global per entity, occurring on first encounter.
   - If any branch version is already selected, that branch executes.
   - Otherwise, first branch executes and its version is recorded.
   - Recorded in `DuraladePatchSelect` event (see engine specification).
1. The `% version complete` form marks a resolved patch.
   - Ensures version is in entity's selected set.
   - Error if replaying and version not selected, or if previously selected branch is removed.

## 10. Expressions

### 10.1. Overview

```wirth
expression = expression_parenthesized |
             expression_access |
             module_access |
             identifier |
             outer |
             unary |
             binary |
             narrowing |
             invocation |
             literal |
             wait |
             spawn |
             anonymous_data |
             anonymous_func .

expression_parenthesized = "(" expression ")" .
expression_access = expression ( "." | "?." | "->" ) identifier .
```

1. Expressions are evaluated to produce values.
1. Literals are defined in Section 10.7, identifiers in Section 5.
1. Parenthesized expressions group sub-expressions to control evaluation order.
1. Access expressions use `.` to access fields or functions on values, or `?.` for nil-safe access.
1. Entity reference access expressions use `->` to access fields or methods on entity references (see Section 7.3).
   - The `->` operator performs blocking calls.
   - Only valid on entity reference types.
1. All expression forms use parenthesized or operator syntax, never space-separated arguments.
   - 💭 Why? Space-separated forms are reserved for statements (e.g., `var`, `return`). Expressions compose inside other expressions, so parenthesized syntax avoids ambiguity and keeps the grammar predictable.
1. Operator precedence from highest to lowest:
   - Parentheses `()`
   - Access `.`, `?.`, and `->`
   - Invocation
   - Postfix unary operators
   - Prefix unary operators
   - Binary operators (detailed in Section 10.4)
1. Additional expression forms may be added in the future.
   - ❓ Ternary conditional expression syntax TBD. Syntax like `condition ? true_expr : false_expr` may conflict with
     existing uses of `?` (nilable types, nil-safe access, nil unwrapping).

### 10.2. Outer Scope Access

```wirth
outer = "outer" { "." "outer" } .
```

1. The `outer` keyword provides access to the immediately enclosing scope.
1. `outer` can be chained to access further enclosing scopes.
   - Example: `outer.outer.field_name`
1. Through `outer`, you can access anything the outer scope provides: fields, functions, variables.
   - Example: `outer.order_id` - access shadowed field
   - Example: `outer.some_func()` - call outer function
1. `outer` itself cannot be accessed as a value (e.g., no `var x = outer`).
   - 💭 Why? `outer` is a scope accessor, not a value. It must be followed by field or function access.

### 10.3. Unary

```wirth
unary_prefix = ( "-" | "!" ) expression .
unary_postfix = expression "!" .
unary = unary_prefix | unary_postfix .
```

1. Unary operators take a single operand.
1. Prefix unary operators appear before the operand.
   - `-` negates numeric values.
   - `!` performs logical NOT on boolean values.
1. Postfix unary operators appear after the operand.
   - `!` is the early return operator, only valid on expressions that return types with an `out!` field.
1. The early return operator `!` triggers early returns based on the `out!` field.
   - Valid on any expression that returns a type with an `out!` field (invocations, anonymous data with `out!`, etc.).
   - If the `out!` field is not nil, the `out!` of the current function is set with the value and then a return occurs.
   - If the `out!` field is nil, execution continues with the normal result.
   - Example: `result := some_func()!` - if `some_func` returns a non-nil `out!` field, the current function's `out!` is
     set and returns; otherwise `result` gets the normal return value.

### 10.4. Binary

```wirth
binary_op = "+" | "-" | "*" | "/" | "%" |
            "==" | "!=" | "<" | "<=" | ">" | ">=" |
            "&&" | "||" | "??" .
binary = expression binary_op expression .
```

1. Binary operators take two operands.
1. Arithmetic operators perform mathematical operations.
   - `+` addition (also string concatenation), `-` subtraction, `*` multiplication, `/` division, `%` modulo
1. Comparison operators compare values and return boolean results.
   - `==` equal, `!=` not equal
   - `<` less than, `<=` less than or equal, `>` greater than, `>=` greater than or equal
1. Logical operators perform boolean operations.
   - `&&` logical AND, `||` logical OR
1. The nil coalescing operator `??` returns the left operand if not nil, otherwise the right operand.
   - Left operand must have type `T?`, right operand must have type `T`. Result type is `T`.
   - Short-circuits: right operand is not evaluated if left operand is not nil.
   - Example: `x ?? default_value`
   - ❓ When union types are added, could allow unrelated right operand type with union or supertype result.
1. Binary operator precedence within binary expressions, from highest to lowest:
   - `*`, `/`, `%` (multiplicative)
   - `+`, `-` (additive)
   - `<`, `<=`, `>`, `>=` (comparison)
   - `==`, `!=` (equality)
   - `&&` (logical AND)
   - `||` (logical OR)
   - `??` (nil coalescing)
1. Binary operators are left-associative.
   - ❓ Bitwise operators (`&`, `|`, `^`, `<<`, `>>`) TBD. Duralade uses unbounded integers, so bitwise operations would
     need defined semantics (e.g., operate on fixed-size representations like 64-bit). For now, bitwise operations can
     be provided as explicit stdlib functions if needed.

### 10.5. Type Narrowing

```wirth
narrowing_nil_default = expression "?" "!" .
narrowing_nil_custom = expression "?" "else!" expression .
narrowing_as_default = expression "as!" type .
narrowing_as_custom = expression "as" type "else!" expression .
narrowing = narrowing_nil_default | narrowing_nil_custom | narrowing_as_default | narrowing_as_custom .
```

1. Type narrowing expressions check a value's type or nil-state and early-return on failure.
1. Form 1: `expr? else! value` - Nil check with explicit `out!` value.
   - If nil, early-returns with `out!` set to the expression.
   - If non-nil, evaluates to the unwrapped (non-nilable) value.
   - Example: `result := entity::run_result(e)? else! error::create("result not ready")`
1. Form 2: `expr?!` - Nil check with default error.
   - Like form 1 but defaults to a generic "value was nil" error, only usable when `out!` is an error type.
   - Example: `result := entity::run_result(e)?!`
1. Form 3: `expr as type else! value` - Type assertion with explicit `out!` value.
   - If not assignable to type, early-returns with `out!` set to the expression.
   - If assignable, evaluates to the value with the narrowed type.
   - Example: `msg := inbox as request_message else! error::create("unexpected message type")`
1. Form 4: `expr as! type` - Type assertion with default error.
   - Like form 3 but defaults to an error including expected and actual types, only usable when `out!` is an error type.
   - Example: `msg := inbox as! request_message`
1. All forms require an `out!` field in the containing function.
1. The `else!` expression must be assignable to the `out!` field's type and is only evaluated on failure.
1. Type narrowing differs from nil coalescing (`??`): `??` provides a fallback and continues, narrowing expects success
   and early-returns on failure.
1. ❓ Exact error messages and types for default forms TBD. Should specify precisely what error data is created for `?!`
   and `as!` forms.

### 10.6. Invocation

```wirth
invocation_argument = [ identifier "=" ] expression .
invocation_arguments = invocation_argument { "," invocation_argument } [ "," ] .
invocation = expression [ type_arguments ] "(" [ invocation_arguments ] ")" .
```

1. Invocation expressions call functions, member functions, or construct entities and data.
1. The expression before `()` determines what is invoked.
   - Identifier: function name or type name for construction
   - Access expression: member function call (e.g., `entity.member_func()`)
1. Type arguments may be provided in square brackets before the argument list (see Section 7.2).
   - Type arguments follow the same shorthand rules as regular arguments.
   - Type arguments may be omitted when all type parameters have defaults.
   - Example: `process[t = int](item = 5)`
1. All arguments are named arguments.
   - Full form: `name = expression`
   - Shorthand form: Only identifier or access expressions can elide the name. The parameter name is inferred from the
     last identifier.
   - Non-identifier/access expressions require the full form: `func(count = 5)`, `func(calculate = calculate())`
   - Arguments must set all required `in`/`inout` fields and can set optional `in`/`inout` fields.
   - Example: `func(x)` is shorthand for `func(x = x)`, `func(data.field)` is shorthand for `func(field = data.field)`
   - 🔒 Spaces are required around `=` in invocations.
   - 💭 Why `=` instead of `:`? For consistency, `=` always means binding a value (or type), while `:` always means type
     annotation. This makes the syntax predictable: `:` for types, `=` for values/assignments.
   - 🔒 When an identifier or access expression's last identifier matches the parameter name, the shorthand form must be
     used.
1. ❓ Argument ordering requirements in strict mode TBD. Matching callee field order is problematic since adding
   defaults moves fields (required before optional), breaking call sites.

### 10.7. Literals

```wirth
literal_number_digit = "0" … "9" .
literal_number_digits = literal_number_digit { [ "_" ] literal_number_digit } .
literal_number_int = literal_number_digits .
literal_number_float = literal_number_digits "." literal_number_digits .
literal_number = literal_number_int | literal_number_float .

literal_string_hex_digit = "0" … "9" | "a" … "f" .
literal_string_escape = "\" ( "n" | "r" | "t" | "\" | `"` |
  "x" literal_string_hex_digit literal_string_hex_digit |
  "u" literal_string_hex_digit literal_string_hex_digit literal_string_hex_digit literal_string_hex_digit |
  "U" literal_string_hex_digit literal_string_hex_digit literal_string_hex_digit literal_string_hex_digit
      literal_string_hex_digit literal_string_hex_digit literal_string_hex_digit literal_string_hex_digit
) .
literal_string_quoted = `"` { unicode_char | literal_string_escape } `"` .

literal_string_raw_delimiter = "``" { "`" } .
literal_string_raw = literal_string_raw_delimiter { unicode_char } literal_string_raw_delimiter .

literal_string = literal_string_quoted | literal_string_raw .

literal_bool = "true" | "false" .

literal_nil = "nil" .

literal_array = "[" [ expression { "," expression } [ "," ] ] "]" .

literal_map_entry = expression "=" expression .
literal_map = "{" [ literal_map_entry { "," literal_map_entry } [ "," ] ] "}" .

literal = literal_number | literal_string | literal_bool | literal_nil | literal_array | literal_map .
```

1. Number literals without a decimal point are integers; with a decimal point are floats.
   - Integer examples: `42`, `1_000_000`
   - Float examples: `3.14`, `1_000.5`
   - Type inference: literals without a decimal point are inferred as `int`, those with a decimal point as `float`.
   - 🔒 Numbers with 4 or more digits must use underscores every 3 digits for readability (e.g., `1_000`, `1_000_000`,
     `3_141.592_653`).
1. Quoted string literals are enclosed in double quotes and support escape sequences.
   - Standard escape sequences: `\n` (newline), `\r` (carriage return), `\t` (tab), `\\` (backslash), `\"` (double
     quote).
   - Hexadecimal escapes: `\xHH` (2 lowercase hex digits for byte value).
   - Unicode escapes: `\uHHHH` (4 lowercase hex digits) and `\UHHHHHHHH` (8 lowercase hex digits).
   - Example: `"hello\nworld"` produces a string with a newline between `hello` and `world`.
   - 💭 Why lowercase hex only? Deterministic formatting requires choosing one canonical form. Lowercase is standard in
     most modern languages.
   - ❓ String interpolation syntax TBD.
1. Raw string literals are enclosed in two or more backticks and have no escape sequences.
   - The opening delimiter is N backticks (N >= 2). The closing delimiter is exactly N backticks.
   - All characters between the delimiters are literal, including backslashes, newlines, and single backticks.
   - To include a sequence of backticks in the content, use a delimiter longer than any backtick sequence in the content.
   - Example: ` ``hello`` ` produces `hello`.
   - Example: ` ``C:\path\file`` ` produces `C:\path\file` (backslashes are literal).
   - Example: ``` `` `SELECT * FROM `users` `` ``` produces `` `SELECT * FROM `users` `` (single backticks are literal).
   - Example: ` ``` has ``two`` backticks ``` ` - use 3-backtick delimiter when content contains ```` `` ````.
   - 💭 Why N-backtick delimiters? This makes raw strings truly "raw" - zero escape sequences are needed, ever. Just
     widen the delimiter to accommodate any content. Single backticks are reserved for raw identifiers (Section 5).
   - ❓ Should multiline strings support automatic indentation handling (e.g., YAML-like indent removal or Scala-like
     `stripIndent`)?
1. Boolean literals are `true` and `false`.
1. The nil literal is `nil` and is only valid for nilable types.
1. Array literals create instances of the stdlib `array[t]` type.
   - Element type is inferred from the expressions.
   - Example: `[1, 2, 3]`, `["a", "b"]`
1. Map literals create instances of the stdlib `map[key, value]` type.
   - Key and value types are inferred from the expressions.
   - Example: `{ "foo" = 1, "bar" = 2 }`

### 10.8. Wait

```wirth
wait = "wait" "(" expression ")" .
```

1. Wait expressions block execution until a boolean condition evaluates to `true`.
1. Returns `{ out! :error? }`.
   - Example: `wait(running < max_concurrent)!`
1. Wait can only be used in blocking contexts (not in `view` or `noblock` functions).
1. The condition must be `view`-only (no side effects), enforced by the compiler.
   - 💭 Why? The condition is evaluated repeatedly on each tick.
1. Wait checks the implicit `cancellation` context, returning `{ out! = canceled_error }` if cancelled.
1. Wait yields execution and is re-evaluated on each tick until satisfied or cancelled.
1. Wait does not support type narrowing or nil unwrap forms.
   - 💭 Why not? Those forms would need to return `{ value: t, out! :error? }`, but `value` would be undefined when
     cancelled. Making `value` nilable defeats the purpose. Use type narrowing expressions (Section 10.5) with `wait`
     instead: `result := wait(expr?)?!`
1. ❓ Timeout support TBD.

### 10.9. Spawn

```wirth
spawn = "spawn" "(" invocation [ "," invocation_arguments ] ")" .
```

1. Spawn creates an independent entity instance.
   - First argument is an entity invocation (entity name with constructor arguments).
   - Additional keyword arguments configure the instance.
   - Example: `ref := spawn(transfer(from_account = "acc1", to_account = "acc2", amount = 100))!.entity`
   - Example with id: `spawn(transfer(from_account = "acc1", to_account = "acc2", amount = 100), id = "xfer1")!`
1. Returns `{ entity: &T?, out! :error? }` where `T` is the entity type.
   - `entity` is the reference to the spawned instance (nil when spawn fails).
   - `out!` is populated when the spawn itself fails (e.g. invalid entity type, runtime refusal).
1. Blocking - cannot be used in `view` or `noblock` functions.
   - 💭 Why? Spawn has durable side effects (creating an independent entity).
1. The spawned entity runs independently from the caller.
   - Use `->` on the returned reference to interact with it (see Section 7.3).
1. Keyword arguments:
   - `id`: optional `str` - caller-chosen instance identifier. Runtime assigns one if omitted.
   - ❓ Additional spawn arguments (priority, affinity, etc.) TBD.
1. ❓ Operator customization of remote-ness TBD - how operators influence whether spawned entities run locally, on a specific node, or are scheduled by a cluster.

### 10.10. Anonymous Functions and Data

```wirth
anonymous_data = "data" "{" { field_signature } "}" .
anonymous_func = [ "view" | "noblock" ] "func" "{" func_body "}" .
```

1. Anonymous data expressions create instances of anonymous data types.
   - Uses the same field syntax as named data constructs (`field_signature`, newline-separated; see Section 8.4).
   - Every field must have a `= expression` default (since an instance is being created, every field needs a value).
   - Field types are inferred from the expression if not explicitly provided.
   - Example:
     ```
     data {
       x = 1
       y: str = "hello"
     }
     ```
1. Anonymous data field shorthand applies when the field name matches an identifier in scope.
   - Example: `data { x }` is shorthand for `data { x = x }` when `x` is in scope.
   - 🔒 When an identifier matches the field name, the shorthand form must be used.
1. Anonymous function expressions create function instances.
   - Use the same body structure as named functions (see Section 8.6).
   - Can specify `view` or `noblock` modifiers just like named functions.
   - Type inference applies to field types as with variable declarations.
   - ❓ Shorthand syntax for anonymous functions (lambda-style) is TBD.
1. Anonymous expressions evaluate to instances of their corresponding anonymous types (see Section 7.5).
