# Overview

Duralade is a strongly typed language meant to run durable software, meaning the language is meant for code that can run
for years. Specific language features include:

* Functions and objects are the same thing, called "fob"s (short for function-objects) that have in types (generics),
  in vars (params), out vars (results), out fobs (methods), and more
* Full system snapshot/resume via reflectable object graph
* Built-in primitives for coroutines, waiting for state, patching, etc
* Ability to determine levels of compatibility when comparing source code
* Guaranteed deterministic with ability to make "extern" calls to external things
* Automatic garbage collection (reference counting with cycle detection)

Duralade is built for WebAssembly GC future but in the absence of a good enough implementation, a high-level interpreter
is made available.

TODO:

* Quick start + small sample
* Explainer of each point in the intro