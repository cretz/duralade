# Duralade Conformance Tests

Run by the `duralade-conformance-harness` crate.

## `parser/`

Single-file parse failure tests. Each `.dl` is parsed in isolation and is expected to produce parse/strict diagnostics.
Expected errors are marked with `^` carets aligned under the error span followed by `@@ERROR: message` or `@@STRICT: message` (strict-mode warnings). The harness strips these lines before parsing and matches actual errors by position and message substring.

## `loader/`

Loader pipeline failure tests (imports, types, annotations). Each `.dl` is loaded with stdlib and is expected to produce load diagnostics.
Same `@@ERROR` caret format as parser tests.

## `loader_types/`

Type inference assertion tests. Each `.dl` is loaded with stdlib and must produce zero errors. Expected types are marked with `^` carets aligned under an expression followed by `@@TYPE: typename`. The harness finds the AST node at that exact byte range in the `TypeTable` and compares `Type::display()` output against the expected string.

## `runtime/`

A normal Duralade project with tests. Success-path compatibility tests belong here.

## `project/`

Multi-module project tests. Each subdirectory is a standalone project (`duralade.toml` + `src/` + `test/`). Same `@test` function format as `runtime/`.

- `basic/` - spawns an entity and verifies its result.
