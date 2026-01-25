# Duralade Conformance Test Suite

This directory contains the conformance tests for the Duralade language specification.

These tests are implementation-agnostic - they're pure Duralade code with annotations indicating expected behavior.

## Structure

- `parser/` - Tests for parsing and syntax validation
- `types/` - Tests for type system behavior
- `runtime/` - Tests for runtime execution and semantics

## Test Format

TODO: Define the format for test annotations and expected results.

## Test Harness

The `duralade-conformance-harness` crate provides the test runner and assertion framework.
