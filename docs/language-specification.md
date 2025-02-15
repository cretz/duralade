# Duralade Language Specification 0.1.0

## 1. Language Overview

1. This document is the specification for Duralade language syntax.
   - Specification for the Duralade runtime is in a separate document.
   - The specification version represents the language version, but the version may increase for other changes outside
     of this specification.
   - WARNING: Duralade is not yet stable and therefore nothing here is stable.
1. Brief overview of general Duralade features:
   - Statically/strongly typed, garbage collected.
   - Durable execution language for code that can run years.
   - Engine can be "ticked" to run until all coroutines are yielded awaiting external stimulus.
   - All state is serializable at any time after a "tick".
   - All code is always deterministic, "extern"s are the side effecting pieces.
   - Built-in support for checking whether changes are incompatible with previous code.
1. Brief overview of general Duralade language syntax features, expanded upon later in this document:
   - Code is UTF-8.
   - Constructs and statements are separated by newlines.
   - Comments are `#` prefixed.
1. Code is as deterministic as reasonable.
   - 💭 Why? This is so tooling and humans can easily read/write accurate, shareable code.
1. Strict formatting is part of the spec itself, not a separate tool.
   - Tools may accept a looser format while typing, only show the looser format violations while typing, and
     automatically format on save.
   - This is known as "strict format".
   - 💭 Why? Ensuring strict formatting allows predictability for tooling and humans.

## 1. Specification Format

1. The format of this file is markdown with a maximum of 120 characters per line.
1. Each section is numbered and in each section a numbered bullet list exists with rules.
   - Rules are expected to have punctuation.
   - 💭 Why number so many things? For easy referencing.
1. Numbered rules can have sub-bullets explaining details.
   - Some sub-bullets can have emojis to explain certain aspects. The accepted emojis are:
     - `❓ <question>?` - An outstanding question that can still change behavior before stable.
     - `💭 Why <question>?` - Explains why a decision was made.
     - `🔢 - `- Note about how the feature interacts with code versioning. Specifically, terms "compatible" and
       "incompatible" are used.
     - 🔒 - For bullets that only apply in strict format mode.
   - Sub-bullets should not themselves be numbered in most cases. If such a situation is needed, a new top-level
     numbered rule is ideal.
1. Syntax notation at the top of a section is a variant of
   [Wirth Syntax Notation](https://en.wikipedia.org/wiki/Wirth_syntax_notation).
   - Basically the same as the Go programming language spec.
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
   - The form `a … b` represents the set of characters from a through b as alternatives. The horizontal ellipsis `…` is
     also used elsewhere in the spec to informally denote various enumerations or code snippets that are not further
     specified.

## 1. Source Code

```wirth
newline = "\n" .

unicode_char = /* an arbitrary Unicode code point except newline */ .
unicode_letter = /* a Unicode code point categorized as "Letter" */ .
unicode_lowercase_letter = /* a Unicode code point categorized as "Letter, lowercase" */ .
unicode_digit  = /* a Unicode code point categorized as "Number, decimal digit" */ .

identifier = (unicode_lowercase_letter | "_") { unicode_lowercase_letter | unicode_digit | "_" } .
identifier_qualified = identifier { "." identifier } .
```

1. 🔒 Source code lines should not exceed maximum of 120 lines.
   - Note, this is not a "must" requirement, just a strong recommendation.
1. Source code must be UTF-8 encoded without BOM.
   - 💭 Why without BOM instead of BOM optional? Everything about Duralade is as deterministic as possible, including
     the source file requirements.
1. Identifiers must start with a lowercase letter or underscore, then use all lowercase letters or digits after that.
   - 💭 Why limit to only lowercase and underscores? Identifiers are not meant to support arbitrary casing, even
     code-generated ones. In situations where name customization is required, e.g. JSON formatting based on reflection,
     other approaches should be taken, e.g. having `to_json` functions.

* TODO: Make a form that accepts backticks so there are no keyword restrictions

## 1. Source Files

```wirth
file_construct = type_alias | data | extern | fob | func .
file = { import } { file_construct } .
```

1. Source files represent parts of a Duralade module.
   - ❓ Where do we explain module/project/filename structure?
1. 🔒 There must be a blank line between each section (i.e. two newlines after previous section).
   - Blank lines and comments can exist within each section too if desired.
1. 🔒 File must not start with a newline. File must end with a newline.
1. [Comment](#TODO) block at the beginning is documentation for the module.
   - This is not to be confused with a comment above the first import. The comment must be on its own with a blank line
     after it to be module documentation.
   - Multi-file modules can only have module-level documentation in one file.
     - TODO: Link to this part in the runtime doc.
1. 🔒 [Import](#TODO)s must be in alphabetical order.
1. 🔒 File constructs must be in the following order:
   - Top-level [type alias](#TODO)es, alphabetically.
   - Top-level [data](#TODO), first by `out` vs non-`out`, then alphabetically.
     - Nested constructs ordered as if top-level.
   - [Extern](#TODO)s, alphabetically.
   - Top-level [fob](#TODO)s, first by `out` vs non-`out`, then alphabetically.
     - Nested constructs ordered as if top-level.
   - Top-level [func](#TODO)s, alphabetically.

### 1. Comments

1. Comments start with `#`. Comments can span lines.
1. 🔒 Comments must be the only thing on a line and must start at the expected indentation level.

- 💭 Why not allow trailing comments on source lines? It can be unclear and very subjective when to do this

1. Some comments, depending on their location, may be assumed to be documentation for a construct.

## 1. Imports

- TODO: Explain files are modules, and non-local module access/use is qualified at access site always (like Go)
- TODO: Somewhere explain that fob, func, or data with same name as module can be used as top-level item when imported

## 1. Fields

```wirth
field_modifier = "in" | "inout" | "out" | "shared" | "implicit" .
field_signature =
  (identifier ":" type [ "=" expression ]) |
  (":" type [ "=" expression ]) |
  (identifier "=" expression) .
field = field_modifier [ "!" ] field_signature .
```

1. Instead of an identifier, a field may just be declared as a type and that type name becomes the identifier.
   - This only works for non-generic types. TODO: Right?
   - In a field set, identifier uniqueness rules still apply.
1. Fields in some constructs/situations can have default expressions.
   - A field with a default expression is considered "optional" and without is considered "required".
   * TODO: Nilable is always optional, right?
   - Default expressions can only reference fields above it.
1. A type must be knowable for a field and therefore explicit type can only be omitted if a default expression is
   present.
1. Fields may exist in [`data`](#TODO), [`extern`](#TODO)s, [`fob`](#TODO)s, and [`func`](#TODO)s.
1. Fields must have a single modifier (except in `data` cases where it is implied). How these represent behavior is
   dependent on the construct they are used within.
1. `in` fields mean they are accepted from caller.
   - These are often similar to parameters in other programming languages.
   - Without default expression, they are required from the caller.
   - Values for these fields are usually immutable, see specific construct documentation.
1. `out` fields mean they are values that can be obtained by the caller.
   - These are often similar to return values in other programming languages.
   - A default expression is required.
     - 💭 Why for nilable fields, can't `nil` be assumed? It could be assumed for `out` but not for `in` and therefore
       for clarity/consistency, it is easier to just require for all.
   - Values for these fields are usually mutable, see specific construct documentation.
   - One out fields can have `!` after the modifier (i.e. `out!`) which marks it as the early return field for use with
     `return!` statements and
     - Only allowed on one.
     - Only allowed on fobs and funcs with bodies, has no caller significance.
     - Commonly used with error, e.g. `out! :error`.
1. `inout` fields are a bit of combination of `in` and `out` with a few differences.
   - They behave like `in` fields with regards to default expression.
   - They behave like `out` fields with regards to mutability.
1. `shared` fields are shared amongst nested constructs.
   - These are usually mutable, see specific construct documentation.
   - `in`, `inout`, and `out` fields are implicitly shared amongst nested constructs.
1. `implicit` fields are derived and propagated contextually.
   - These must have a default expression or by nilable.
1. 🔒 Fields must be in the following order:
   - `in` fields with no default expression.
   - `in` fields with default expression.
   - `inout` fields with no default expression.
   - `inout` fields with default expression.
   - `out` fields with no default expression.
   - `out` fields with default expression.
   - `shared` fields.
1. Unlike file-level constructs, fields do not have to be alphabetically ordered.
   - 💭 Why? Field ordering affects default expression availability for the following fields, and field ordering
1. Comments may appear above fields to document them.
1. Field changes can affect versioning.
   - 🔢 Any field removal is incompatible. This includes name change which appears as removal + addition.
   - 🔢 Any required field addition is incompatible, optional field addition is compatible.
   - 🔢 Any field position change is compatible, but it is still subject to other field order rules.
   - 🔢 Any field may add a default expression, but may have to move position subject to other field order rules.
   - 🔢 An `in` field compatible type change is compatible, all other field type changes are incompatible.

## 1. Types

```wirth
type_generic = [ identifier ":" ] type .
type_generics = "[" { type_generic [ "," ] } "]" .
type_anonymous = ("data" | "func" | "fob") "{" { field } "}" .

type = ((identifier_qualified [ type_generics ]) | type_anonymous) [ "?" ] .

type_alias = [ "out" ] "type" identifier_qualified "=" type .
```

1. Types can be a named reference to an existing `data` or `fob` construct.
   - 💭 Why not an existing `func` or `extern`? Anonymous types with the function signature are preferred in these
     scenarios.
1. Types can be anonymous unnamed types.
   - `data` types mean an instance that is compatible with the given field set.
     - TODO: Clarify compatibility here, mostly around covariance.
   - `func` types mean a callable type compatible with the given field set.
   - `fob` types mean a startable fob compatible with the given field set.
   - ❓ Support
1. Types can have generics.
1. Type aliases are literally replacements for the type at runtime and are just for keeping naming simple.

TODO:

- Unions?
- TODO: Explain nullability and `?`
- TODO: Generics?
  - Ideally there are `intype` type of fields that are required before all other fields
  - These are a must have at this point so users can make their own collections
  - The type of the `intype` is the upper bound (how to do contra/covariance?)
    - Type is not required and is assumed to have upper bound of `any?`
- Interfaces?
  - Error could arguably be one such interface
- `any` type is for anything
  - TODO: Safe casting?
  - `any` is not nullable, only `any?` is
- Ways to say `data`, `fob`, or `func`?
- Unions? Type aliases? Instance of checks?
- TODO: Super/sub types?

## 1. Data

```wirth
data_modifier = "out" .
data = [ data_modifier ] "data" identifier "{" { field_signature } "}" .
```

1. Data is a named construct for serializable fields. No constructor logic.
   - It is common to have a top-level `new` or a more specific `new_thing` as a constructor.
   - 💭 Why no constructors on data? To be compatible, serializable, and have clear mutability, it is clearer if all
     things that work with data are not "on" the data.
   - 💭 Why not support marking some fields as serializable and some not? Because then the ability to customize
     (de)serialization would be required making the comprehension of the system complex. It is clearer to encourage
     authors to treat serializability as all or none.
1. Data fields must all be serializable types.
   - TODO: Explain type serializability somewhere
1. `out` data means external users can see/use the data.
   - 🔢 Removing `out` is incompatible, adding `out` is compatible.
1. Data fields cannot have modifiers, they are implicitly `inout`, meaning required fields must be set on creation, but
   optional ones don't have to be.
   - 💭 Why no "private" data fields/vars? For data to remain clear to users and serializable, it encourages better
     practice to require authors to treat all state as visible/serializable and separate accessibility needs at a higher
     level.
1. Whether data fields are mutable is dependent upon the situation in which they are used.
1. Data can have nested `func`s and `fob`s.
   - 💭 Why? Having helpers access data as shared state
1. Data are always equatable and hashable.

- TODO: Provide a concept of kwarg splatting (both callsite and accepting defn site)

## 1. Fobs

```wirth
fob_modifier = "out" .
fob_body = { field_modifier field } { statement } .
fob = [ fob_modifier ] "fob" identifier "{" fob_body "}" .
```

1. Fobs (aka "function objects") are asynchronous functions/objects that can be started and interacted with.
1. Out fields can be accessed at any time, even before complete.
1. Can have nested funcs and fobs that can access all fields.
1. `out` fob means external users can see/use the fob.
   - Requires all `in` and `out` fields to be `data` and visible themselves.
   - 🔢 Removing `out` is incompatible, adding `out` is compatible.

- TODO: Cancellation
  - Will probably support an async construct called a latch that is one-way, hierarchical, and just make that the
    encouraged cancel approach
  - Well, the above is not a good idea because you can't really pass around a latch
  - Now thinking there should be a new field type called `insignal` that is a unidirectional channel that queues
    - But cancel can only be called once and it is a state, not an enqueued item
  - Now thinking about an `incancel`. So cancels will implicitly propagate, but `incancel` will give you access
    - Would also need ability to create new, linked or detached, that can invoke cancel
    - Can this be an async local instead? So a runtime only thing?
      - This is kinda annoying without global vars
      - Maybe a new field construct like `implicit` or `context`? That encourages its use though.
        - Still may be worth it, and you can only have one `implicit` of a type (field name does not matter).
        - Implicit must be declared as a field (i.e. after shared)
        - What are versioning expectations here?
        - Does `implicit` have to be serializable for an out fob?
- TODO: Explain `run` and `start`, a start handle, etc
- TODO: Errors?
  - Originally considered `out :error` as any other

## 1. Externs

```wirth
extern_body = { field_modifier field } .
extern = "extern" identifier "{" extern_body "}" .
```

1. Externs are contracts for externally-defined, side-effecting, async functions.
1. Started/run and canceled like fobs, but cannot be interacted with during execution.
1. Externs cannot be marked "out", rather they are never external to their module, they must be wrapped to be exposed.

- TODO: Cancellation
- TODO: How are externs best built, in host lang? Sidecar?
  - Should there be an extern protocol, e.g. on top of HTTP?
  - Should there be an extern C FFI?
    - Most definitely. DLLs are a good way here
  - Need a way to validate extern signatures at runtime
    - Can MCP help here?

## 1. Funcs

```wirth
func_modifier = "out" | "view" .
func_body = { field_modifier field } { statement } .
func = { func_modifier } "func" identifier "{" func_body "}" .
```

1. Funcs (aka "functions") are like fobs, but cannot be interacted with, they simply start and complete.
1. Funcs cannot have anything nested beneath them.
1. `out` func means external users can see/use the func.
   - Requires all `in` and `out` fields to be `data` and visible themselves.
   - 🔢 Removing `out` is incompatible, adding `out` is compatible.
1. `view` funcs are read-only, immediately returning functions.
   - Cannot mutate anything outside of themselves.
   - Can only call other view funcs, no fobs, non-view funcs, or externs.
   - Cannot use any async constructs like `wait` and `start`.

## 1. Patches

```wirth
patch_begin_def = ( "default" | string_literal ) "{{" .
patch_begin = "%" patch_begin_def .
patch_end = "%" "}}" [ patch_begin_def ] .
patch_line = patch_begin | patch_end .
```

- TODO: Explain this
- TODO: Explain these are pre-compiler directives and therefore do not fit in with traditional AST

## 1. Callables

- TODO: Explain these can't be defined as in or out fields on an out construct since not serializable
- TODO: Or is that too limiting?

## 1. Errors

- TODO: We have `return!` that, if the expression is nil does nothing, but if it is non-nil sets `out!` as that value
  and returns
- TODO: We have `!` unary postfix that extracts same-named field out and propagates it and returns. It is shortcut for
  just assigning to `tmp` var, doing `return! tmp.<whatever out! is named>`, then completing expression as `tmp`.

## 1. Cancellation

- TODO: Explain it's just an implicit (should this get moved out of this guide since it's not syntax)

## 1. Statements

```wirth
block = { "{" statement "}" } .
statement = 
  block |
  local_variable |
  for |
  assignment |
  if |
  wait |
  defer |
  return |
  implicitly |
  expression .
```

- TODO: Explain statement termination
  - Go rules are a bit constricting and Python/Ruby/JS rules are a bit rough too. Our rule will simply be "if the
    trailing item at newline can end the statement it does" (which means method chaining requires dot before newline).
    We also have to add caveat that we don't support statements starting with parentheses so that people don't get
    confused.
- TODO: Explain blocks
- TODO: Panic?
- TODO: Yield? This would be for CPU bound work that effectively wants to give up control for distribution purposes
    * This would have to be some kind of construct that yields to itself iterative so we can memoize something to
    continue where it left off instead of replaying

### 1. Local Variable Statements

```wirth
local_variable = "var" field_signature .
```

TODO: Link to assignment
TODO: Explain the expressions, defaults, etc match fields
TODO: Explain shadowing rules

### 1. For Statements

```wirth
for_label = identifier ":" .
for_traditional = [ statement ] ";" [ expression ] ";" [ statement ] .
for_in = identifier "in" expression .
for = [ for_label ] "for" [ expression | for_traditional | for_in ] block .
break = "break" [ ":" identifier ] .
continue = "continue" [ ":" identifier ] .
```

- TODO: Explain versioning and such

### 1. Assignment Statements

```wirth
assignment_op = "=" | ":="
assignment = expression assignment_op expression .
```

* TODO: Multi-assign and/or destructuring?
* TODO: `binary_op "="` form?

### 1. If Statements

```wirth
if = "if" [ statement ";" ] expression block { "else" (if_statement | block) } .
```

* TODO

### 1. Wait Statements

```wirth
wait = "wait" expression .
```

* TODO

### 1. Defer Statements

```wirth
defer = "defer" block .
```

* TODO

### 1. Return Statements

```wirth
return_always = "return" .
return_early = "return!" expression .
return = return_always | return_early .
```

* TODO

### 1. Implicitly Statements

```wirth
implicitly = "implicitly" (assignment | expression) [ block ] .
```

* TODO

## 1. Expressions

```wirth
expression =
  literal |
  identifier |
  invoke |
  start |
  unary |
  binary |
  parenthesized |
  anonymous .
```

- TODO: Need "outer." to disambiguate potentially shadowed outer vars

### 1. Literal Expressions

* TODO

### 1. Invoke Expressions

```wirth
invoke_argument = (identifier [ ":" expression ]) | expression .
invoke_arguments = "(" invoke_argument { "," invoke_argument } [","] ")"
invoke = expression "(" [ invoke_arguments ] ")" .
```

* TODO

### 1. Start Expressions

```wirth
start = "start" expression .
```

* TODO

### 1. Unary Expressions

```wirth
unary_prefix = ("!" | "-") expression .
unary_postfix = expression "!" .
unary = unary_prefix | unary_postfix .
```

* TODO

### 1. Binary Expressions

TODO: `+`, `-`, `*`, `/`, `.`, `?.`

### 1. Parenthesized

```wirth
parenthesized = "(" expression + ")" .
```

* TODO: Explain this can't be used as a top-level statement (but can be used as first part of if statement)

### 1. Anonymous

```wirth
anonymous_fob = "fob" "{" fob_body "}" .
anonymous_func = "func" "{" func_body "}" .
anonymous = anonymous_fob | anonymous_func .
```

* TODO