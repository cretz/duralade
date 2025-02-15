# Language Specification

This document is the specification for Duralade language.

NOTE: This is under dev and is missing many things

## Syntax Notation

Syntax notation in this doc is defined in a variant of
[Wirth Syntax Notation](https://en.wikipedia.org/wiki/Wirth_syntax_notation) (basically the same as the Go spec). Here
it is defined in itself:

```
syntax      = { production } .
production  = production_name "=" [ expression ] "." .
expression  = term { "|" term } .
term        = factor { factor } .
factor      = production_name | token [ "…" token ] | group | option | repetition .
group       = "(" expression ")" .
option      = "[" expression "]" .
repetition  = "{" expression "}" .
```

Productions are expressions constructed from terms and the following operators, in increasing precedence:

```
|   alternation
()  grouping
[]  option (0 or 1 times)
{}  repetition (0 to n times)
```

Lexical tokens are enclosed in double quotes "" or backticks ``.

The form a ... b represents the set of characters from a through b as alternatives. The horizontal ellipsis ... is also
used elsewhere in the spec to informally denote various enumerations or code snippets that are not further specified.

## Source Files

```wirth
file = { import } | { fob } .
import = "import" { ident "." } ident [ "as" ident ] .
```

* UTF-8 (BOM allowed at beginning only, but not required, discouraged)
* Only `\n` and `\s` valid outside of constructs, `\r` and `\t` are invalid
* `.dl` is file extension

### Comments

* `#` starts comment
* Everything after comment until end of line is part of comment

### Identifiers

```wirth
letter = "A" ... "Z" | "a" ... "z" | "_" .
digit = "0" ... "9" .
ident = letter { letter | digit } .

type = ident [ type_args ] [ "?" ] .
type_args = "[" type_arg { "," type_arg } [ "," ] "]" .
type_arg = ident ":" type .
```

* All lowercase snake_case is encouraged for all identifiers
* The single `_` means ignore in some places and cannot be used in others
* Type args, like parameters, have a name, hence the ident
* TODO: Clarify reserved keywords cannot be identifiers because we can't tell keyword + paren expr from fob invocation

### Imports

```wirth
import = "import" { ident "." } ident [ "as" ident ] .
```

* See runtime for how files map to modules
* Trailing ident of import, or `as` name, is how the module is accessed (i.e. it's the prefix)
  * If module has `out fob` of same name, it can be invoked as the module reference itself 
* Every import module name or alias must be unique


## Statements

```wirth
stmt = fob |
       out_var |
       local_var |
       assign |
       for |
       return |
       wait |
       defer |
       expr .
```

### Fobs (i.e. Function Objects)

```wirth
fob_mod = "extern" | "out" | [ "out" ] "view" .
fob = [ fob_mod ] ( "fob" | "ifob" ) ident "{" fob_body "}" .
fob_body = in { stmt } .
```

* Fobs are "called" or "started" and contain inner fobs/vars and sometimes other statements
  * Fobs must define all in types before in vars, and those before all other statements
* Extern fobs are implemented externally and have memoized results
  * Only in var/type, out var and out fob statements allowed
  * Can not be marked "out" for consumption be callers
* Out fobs are accessible outside the module
  * An out fob of the same name as the file/module can be called on the import alias directly
* View fobs cannot do any mutations
  * Cannot be defined at top level
  * These can be accessed even when outer fob done, whereas non-view fobs one accessible before done
  * Can be used in other view fobs
  * Can be used in waits
* Ifobs are for defining duck types
  * Have same statement restrictions as extern fobs
* TODO: Somewhere explain utilities for equality, hashing, and string/json conversion, etc. They should all be
  reflection based which means a full/clear reflect system is needed
* TODO: How to use ifobs for sum types?
* TODO: How to ensure a fob satisfies an ifob?

### Variables

```wirth
var_sig_single = (ident ":" type) | (ident [":" type] "=" expr) | ( ":" type ) .
var_sig_multi = "{" { var_sig_single } "}" .
var_sig = var_sig_single | var_sig_multi . 

in = { in_type } { in_var } .

in_type_single = "intype" ident ["=" type] .
in_type_multi = "intype" "{" { in_type_single } "}"
in_type = in_type_single | in_type_multi .

in_var = "in" ["out" | "outmut"] var_sig .
out_var = ("out" | "outmut") var_sig .
local_var = "var" var_sig .
```

* Vars can infer types or names
  * Vars can have expression to initialize with that infers type from the expression
* In vars may have default expressions
  * Without default they are required to be set
  * Without default they must have type declared
  * With default they don't _have_ to define a type, but they can
* Non-in vars may have initializer expressions
  * If they don't, the type must be defined and it must be optional (i.e. with "?" at the end) except if they are in
    ifobs or extern fobs
* Out vars declared after any non-var-decl statements must have optional types
  * This is because they are nil when accessed before they are declared
* Vars without default/initializer expressions must have type
* Vars that define a type don't have to define an ident if it's the same name as the type
* All vars are mutable within the fob
* For vars to be mutable outside the fob, they must be "outmut"
* In types do not have to be set by callers if there is a default or it is one of the types of the in vars
  * Types are reified, i.e. they are accessible as normal vars
  * TODO: Need to think about constraints on in types
* Out vars must be set at some point in the fob (or inner fob)

### Assignments

```wirth
assign = ident "=" expr .
```

* Assignment is a statement, it has no type
* TODO: Type ascription?

### Loops

```wirth
for_label = ident ":" .
for = [ for_label ] "for" [ [ ident "in" ] expr ] "{" { stmt } "}" .
break = "break" [ ":" ident ] .
continue = "continue" [ ":" ident ] .
```

* For loops can have labels that can be provided to break/continue statements
  * TODO: Should we move the label after the `for` to keep our idea of keyword-first to help parser

### Returns

```wirth
return = "return" .
```

* Values are not returned like other languages

### Waits

```wirth
wait = "wait" expr .
```

* TODO: Would rather wait as an expr and have falsy be nil or false only like Ruby?

### Defers

```wirth
defer = "defer" "{" { stmt } "}" .
```

## Expressions

```wirth
expr = literal |
       if |
       patch |
       invoke |
       start |
       done |
       access |
       unary |
       binary |
       paren |
       outer .
```

* TODO: Need a way to ascribe type? E.g. `: type`

### Literals

TODO: Syntax

* Numbers
  * TODO: Integers and floats, with underscores
  * No octals or binary or hex for now
* Strings
  * No such thing as character literals
  * Ruby style (single quote unless interpolation/escape needed)
* Nil

### Ifs

```wirth
if = "if" expr "{" { stmt } "}" { "else" if } [ "else" "{" { stmt } "}" ] .
```

* The type of this expression is a supertype of each arm

### Patches

```wirth
patch_ident = letter { letter | digit | "." } .
patch_qual = paren
patch = "patch" patch_ident [ patch_qual ] "{" { stmt } "}" [ "else" "{" { stmt } "}" ] .
```

* Patches are memoized when first seen
  * `patch_qual` is a read-only expression that if present is what is memoized by, which allows iteration patches
TODO: Is there an "else patch"?
TODO: Explain that patch_qual is a qualifier, so it can be used to be iteration specific, and patch is memoized by ident
  + qual

### Invocations

```wirth

invoke = type invoke_args .
invoke_args = "(" invoke_arg { "," invoke_arg } [ "," ] ")" .
invoke_arg = ident ":" [ expr ] .
```

### Starts

```wirth
start = "start" ( invoke | anon_fob ) .
anon_fob = "fob" "{" { stmt } "}" .
done = "done" expr .
```

### Accessing

```wirth
access = expr [ "?" ] "." ident
```

### Unary Operations

```wirth
unary = bubble_error |
        negative |
        not .
bubble_error = expr "!" .
negative = "-" expr .
not = "!" expr .
```

TODO: Confirm operator precedent with negative and minus, or just remove negative in favor of `* -1`

### Binary Operations

```wirth
binary_op = TODO
binary = expr binary_op expr
```

### Accessing Outer Fob

```wirth
outer = "outer"
```

----------------- OLDER -----------------

### Numbers

* TODO: Integers and floats, with underscores
* No octals or binary or hex for now

### Strings

* No such thing as character literals
* Ruby style (single quote unless interpolation/escape needed)
* TODO: More

## Operators

* TODO
* Basically all WASM operators and + for strings
* No custom operators

## Arrays

* TODO
* Ideally these are just `arr` fobs

## Fobs

Fobs are function-objects and are what is "called" in Duralade.

### Definition

```wirth
fob_mod = "extern" | "out" | [out] "view"
fob = [fob_mod] ("fob" | "ifob") ident "{" fob_body "}".
fob_body = in { stmt }

type = ident [type_args]
type_args = "[" type_arg { "," type_arg } [","] "]"
type_arg = ident ":" type

var_sig = (ident ":" type) | (ident [":" type] "=" expr)

in = { in_type } { in_var }
in_type = "in" "type" ident ["=" type]
in_var = "in" ["out"] var_sig

local_var = "var" var_sig
out_var = "out" var_sig
```

* Fobs:
  * Can be nested, known as an "inner fob"
  * In and out vars cannot be shadowed and are scoped to entire fob regardless of where defined
  * Inner fobs:
    * Must be in fob block, not in some nested block (e.g. if block)
    * Cannot be extern (TODO: or can it when defining an interface?)
  * Extern:
    * Can only have in types, in vars, out vars, and out extern fobs
    * While extern does usually mean externally implemented (i.e. outside the system), can also be used as an interface
      * TODO: What are the problems combining these concepts?
  * Out fobs:
    * Are "exported" to be able to be called from outside
  * View fobs:
    * Cannot mutate anything outside of itself
    * Cannot be "start"ed
    * Cannot call externs or "wait" or invoke any non-view fobs
* Types:
  * Fobs or primitives
  * TODO: Anonymous fob type? Could be useful for lambdas. Maybe more anonymous extern fob type.
  * In types:
    * Basically generics
    * Must be before in vars and must be before body
    * Can be any order (regardless of whether they have defaults) but ones with defaults that rely on other ones in the
      list must be after the ones they rely on
    * TODO: Constraints? (co/contravariance, bounds, etc?)
* Vars:
  * Scoped to entire fob including inner
  * Cannot be shadowed in same fob
  * When accessed before set/defined, zero value
  * In vars:
    * Must be before body
    * Can be any order (regardless of whether they have defaults)
    * Ones with defaults that rely on others in list must be after the ones they rely on
    * TODO: Varargs
  * Out vars:
    * Can be anywhere
    * Can be redefined if there's a common supertype
      * TODO: Right? Or should it require single definition and only assignment after that?
  * Local vars:
    * Can be anywhere

### Statements

```
stmt = local_var |
       out_var |
       fob |
       assign |
       for |
       if |
       patch |
       wait |
       expr

assign = ident "=" expr

label = ident ":"
labeled = label stmt

for = "for" [[ident "in"] expr] "{" { stmt } "}"

if = "if" expr "{" { stmt } "}"

patch = "patch" [":" ident] ident "{" { stmt } "}"

wait = "wait" expr

break = "break" [":" ident]

continue = "continue" [":" ident]

return = "return"
```

* For:
  * No expr means infinite
  * No "in" clause just expr is a while
* Patch:
  * A "for" patch means it is unique per iteration
* Wait:
  * Expression subject to same rules as view fob

### Expressions

```
expr = if_else |
       patch_else |
       invoke |
       start |
       done |
       access |
       bubble_error |
       unary |
       binary |
       paren |
       outer

if_else = if { "else" if } "else" "{" { stmt } "}"

patch_else = patch { "else" patch } "else" "{" { stmt } "}"

invoke = type invoke_args
invoke_args = "(" invoke_arg { "," invoke_arg } [","] ")"
invoke_arg = ident ":" [expr]

start = "start" (invoke | "fob" "{" { stmt } "}")

done = "done" expr

access = expr "." ident

bubble_error = expr "?"

unary = unary_operator expr
unary_operator = TODO

binary = expr binary_operator expr
binary_operator = TODO

paren = "(" expr ")"

outer = "outer"
```

* Invoke:
* Start:
  * Returns fob after first yield point (i.e. extern call)
* Done:
  * True if done, false if not
  * Only works with fob types, not primitives
* Bubble error
  * Wait until given fob done and if `error` out set, set it on current fob and return
* Outer:
  * Access outer fob

### Types

* TODO

### TODO

* Every object can be cloned, both shallow and deep
  * Allow some objects to disallow cloning of themselves?
    * Copy-on-write for single field change?
  * Explain every object is by reference always
* Force formatting
* Explain no constants
* Enum? Sum types? Switch/match?
* Aliases?
* Equality?
* Tuples?
* Rest params?
* Lambdas/blocks?
* Inheritance?
* Goto?
* Defer/finally/ensure?
* Stdlib stuff:
  * Futures, channels, mutexes, etc
  * Optional? Array?
  * Policies (retry, timeout, etc)
    * Policy requires a timeout?
    * Timeouts are stdlib as part of extern policy, sure, but should there be a default somehow? Maybe an analyzer can
      enforce it when default policy created 
  * Cancellation?
    * Thinking async local utility exists and cancel leverages it
    * Would this be better termed "interrupt"?
* Model-only objects? Or some way to say "in out" easier?
  * Grouping, e.g. `"in out {"` and every line is a new var?
* Does file name or dir name define module?
* Decorator or other reflective metadata (i.e. doesn't have to completely wrap like confusing decorator)
* Select?
  * Yes, either you "start" something from before or you "invoke"/"wait" inline, but how to get result?
* Fob of same name as file available as that item
* Match/destructuring
* Error cause best practices?