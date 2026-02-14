# Duralade Standard Library Notes

This document contains design notes and API documentation for the Duralade standard library.

## Table of Contents

1. [Overview](#1-overview)

## 1. Overview

### Prelude and Built-in Modules

The standard library consists of modules under the `duralade` namespace. A blessed set of these modules form the "prelude" - automatically available without explicit imports.

- All stdlib modules are sub-modules of `duralade` (e.g., `duralade.array`, `duralade.map`)
- Prelude modules are implicitly imported - their declarations are available without qualification
- Non-prelude modules require explicit import: `import duralade.assert`
- Modules can be explicitly imported even if in prelude for clarity: `import duralade.error`
- TODO: Projects can opt out of prelude via `duralade.toml` configuration

#### Prelude modules

Only `duralade.error` is in the prelude. This makes the `error` type and `error.simple`/`error.fail` available everywhere without imports, since error handling is fundamental to the language.

All other stdlib modules (assert, test, array, map, str, etc.) require explicit imports.

### Naming Conventions

- Module names: lowercase, singular (e.g., `str` not `string`, `array` not `arrays`)
- Function names: `<noun>_<verb>` (e.g., `array.from_iter`, `str.split`)
- Type names follow same convention as user code (lowercase_snake_case)
- Parameter names: should be the name of the type if possible (e.g., `key` for type `key`, `value` for type `value`), or match what users will commonly pass as identifiers (enables shorthand syntax)

### Design Philosophy

TODO: Minimal but complete, composable, prefer small focused functions

### TODOs

- TODO: Revisit parameter names across all modules following the naming convention (use type names or common identifiers)
- Document nilable ambiguity for: array.get, array.remove, map.get, map.remove

## 2. Modules

- array
  - `data array[t]`
    - `view func concat(other: array[t]) -> array[t]`
    - `view func get(index: int) -> t?` - TODO: doc nilable ambiguity (nil element vs out of bounds)
    - `view func index_of(value: t) -> int?`
    - `func insert(index: int, value: t)`
    - `view func iter() -> iter[t]`
    - `view func len() -> int`
    - `func pop() -> t?` - Remove and return last element, nil if empty
    - `func push(value: t)` - Append element to end
    - `func remove(index: int) -> t?` - TODO: doc nilable ambiguity (nil element vs not found)
    - `func set(index: int, value: t)`
    - `view func slice(start: int, end: int?) -> array[t]`
  - `view func from_iter[t](:iter[t]) -> array[t]`
- bool
  - `data bool`
- cancellation
  - `out data canceled_error { message: str }`
  - `entity cancellation`
    - `out canceled: bool`
    - `out view func check() { out! error: canceled_error? }`
  - `func create(parent: cancellation? = nil) { out :cancellation, out cancel: func { } }`
  - `noblock func detached() -> cancellation` - Creates a new cancellation entity with no parent
- entity
  - `view func run_result[intype e.run@typeout](e) -> e.run@typeout?` - Returns run result if completed, nil otherwise
- error
  - `type error = data { message: str }`
  - `view func create(message: str) -> error`
- float
  - `data float`
    - `view func abs() -> float`
    - `view func ceil() -> int`
    - `view func floor() -> int`
    - `view func max(other: float) -> float`
    - `view func min(other: float) -> float`
    - `view func pow(exp: float) -> float`
    - `view func round() -> int`
    - `view func sqrt() -> float`
    - `view func trunc() -> int`
  - `view func from_int(value: int) -> float`
  - `view func from_str(value: str) -> float?`
- http
  - TODO
- int
  - `data int`
    - `view func abs() -> int`
    - `view func max(other: int) -> int`
    - `view func min(other: int) -> int`
    - `view func pow(exp: int) -> int`
  - `view func from_str(value: str) -> int?`
- iter
  - `type iter[intype t] = func { in :yielder[t] }`
  - `type yielder[intype t] = func { in value: t, out continue: bool }`
- json
  - TODO
- map
  - `data map[key, value]`
    - `view func contains_key(:key) -> bool`
    - `view func get(:key) -> value?` - TODO: doc nilable ambiguity (nil value vs key not found)
    - `view func iter() -> iter[t = data { :key, :value }]`
    - `view func keys_iter() -> iter[t = key]`
    - `view func len() -> int`
    - `func remove(:key) -> value?` - TODO: doc nilable ambiguity (nil value vs key not found)
    - `func set(:key, :value)`
    - `view func values_iter() -> iter[t = value]`
  - `view func from_iter[key, value](:iter[t = data { :key, :value }]) -> map[key, value]`
- random
  - TODO
- reflect
  - TODO
- str
  - `data str`
    - `view func bytes_iter() -> iter[t = int]`
    - `view func ends_with(suffix: str) -> bool`
    - `view func get(index: int) -> str?`
    - `view func index_of(substring: str) -> int?`
    - `view func iter() -> iter[t = str]`
    - `view func len() -> int`
    - `view func replace(old: str, new: str) -> str`
    - `view func slice(start: int, end: int?) -> str`
    - `view func split(delimiter: str) -> array[str]`
    - `view func starts_with(prefix: str) -> bool`
    - `view func to_lower() -> str`
    - `view func to_upper() -> str`
    - `view func trim(all: str?, left: str?, right: str?) -> str`
  - `view func from_float(value: float) -> str`
  - `view func from_int(value: int) -> str`
  - `view func join(:iter[t = str], separator: str) -> str`
- task
  - `entity task[intype t = work@typeout]`
    - `in work: func { }`
    - `out result: t?`
    - `run { out result: t? }`
  - `func completed_iter[t](tasks: array[task[t]]) -> iter[t = data { index: int, result: t }]` - Yields tasks as they
    complete
- test
  - TODO
- time
  - `data duration`
    - `view func add(:duration) -> duration`
    - `view func sub(:duration) -> duration`
    - `view func to_days() -> int`
    - `view func to_days_float() -> float`
    - `view func to_hours() -> int`
    - `view func to_hours_float() -> float`
    - `view func to_millis() -> int`
    - `view func to_millis_float() -> float`
    - `view func to_mins() -> int`
    - `view func to_mins_float() -> float`
    - `view func to_nanos() -> int`
    - `view func to_secs() -> int`
    - `view func to_secs_float() -> float`
  - `data instant`
    - `view func add(:duration) { out :instant?, out! :error? }`
    - `view func elapsed(since: instant) -> duration`
    - `view func sub(:duration) { out :instant?, out! :error? }`
  - `data system_time`
    - `view func add(:duration) { out :system_time?, out! :error? }`
    - `view func elapsed(since: system_time) { out :duration?, out! :error? }`
    - `view func sub(:duration) { out :system_time?, out! :error? }`
  - `view func duration_create(days: int?, hours: int?, mins: int?, secs: int?, millis: int?, nanos: int?) -> duration`
  - `view func instant_current() -> instant`
  - `func sleep(:duration) { out! :error? }`
  - `view func system_time_current() -> system_time`
  - `view func unix_epoch() -> system_time`
