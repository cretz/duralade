# Runtime Specification

This document is the specification for Duralade runtime.

NOTE: This is under dev and is missing many things

## Modules

* File names up until the first dot are module names
  * So `foo.bar.dl` and `foo.baz.dl` are the same `foo` module
* Fob of the same name as module is part of the import
* TODO: How to say what is the primary module of a project?
  * Maybe file alongside `duralade.toml` _must_ be the module that is the name of the file
  * Maybe it's the only module allowed at the root and all others must be under a dir of the same name

## Projects

* `duralade.toml` file defines project
  * Has short name it is defined by and imported as
  * TODO: Dependency management is here with ability to override name, use git or local, leverage tags, use subdir, etc
  * Default has code in `src` and tests in `tests`
  * A `duralade.toml` file nested means it's a completely separate project
  * Maybe the project spec should be in the same language as code? Nah

## Code Versioning

TODO: Explain the details how the check of two typed ASTs can determine patching is needed. This should almost entirely
center around invocations of externs, but not the input contents. So adding, removing, or changing the
condition/loop/etc around something that invokes an extern is incompatible. This has to be transitive, meaning even if
a fob doesn't change, if its input affects when an extern is called, if the outside changes how it populates that, it's
incompatible. Other obvious incompatibilities include changing return value. Maybe not adding a non-default input unless
it violates the above (e.g. affects whether an extern is called). Analysis needs to not care whether something is
refactored into a fob or the inverse (e.g. inlining/deinlining).

## Analysis

TODO:

* Requiring view fobs marked as such
* 