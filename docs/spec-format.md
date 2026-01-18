# Specification Format Guidelines

This document describes the formatting conventions used when writing Duralade specification documents.

## Table of Contents

1. [Document Structure](#1-document-structure)
2. [Numbered Rules](#2-numbered-rules)
3. [Editorial Annotations](#3-editorial-annotations)

## 1. Document Structure

1. Specification documents are written in markdown with a maximum of 120 characters per line.
1. Each specification document must include a table of contents after the title.
   - The table of contents should link to major sections.
1. Each section is numbered and contains a numbered bullet list of rules.
   - Rules are expected to have punctuation.
   - Numbered rules can be referenced in parse errors, format errors, and documentation (e.g., "violates rule 3.4").

## 2. Numbered Rules

1. Each numbered top-level bullet must be a reasonable, separate rule.
   - Numbered rules are referenced by number, so each should be independently meaningful.
   - If a concept requires multiple points, prefer multiple numbered rules over complex nested sub-bullets.
1. Numbered rules can have sub-bullets explaining details.
   - Sub-bullets provide context, examples, or clarifications for the numbered rule.
   - Sub-bullets should not themselves be numbered in most cases. If such a situation is needed, a new top-level
     numbered rule is ideal.
1. Examples must be prefixed with `Example:` (singular) and placed at the bottom of sub-bullet lists.
   - Examples appear after all explanatory sub-bullets but before editorial annotation bullets (emojis).
   - Multiple examples can be listed in a single bullet or consecutively.
   - Always use `Example:` even when showing multiple examples.

## 3. Editorial Annotations

1. Sub-bullets can have emojis to explain certain aspects.
   - `❓ <question>?` - An outstanding question that can still change behavior before stable.
   - `💭 Why <question>?` - Explains why a design decision was made.
   - `🔢` - Note about how the feature interacts with code versioning. Specifically, terms "compatible" and
     "incompatible" are used.
   - `🔒` - For bullets that only apply in strict format mode.
1. Emoji bullets must be the last items in their bullet list at whatever depth they appear.
1. Emoji bullets should not appear on numbered rules themselves, only on sub-bullets.
