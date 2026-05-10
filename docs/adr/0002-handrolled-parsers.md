# ADR-0002 — Hand-rolled parsers (no `nom` / `pest`)

**Status:** Accepted

## Context

Five formats are parsed in this workspace: GDSII (binary), OASIS
(binary, modal-variable + CBLOCK), LEF (text, recursive-descent
sections), DEF (text, similar to LEF), Liberty (text, nested groups
with verbatim payloads), SDF (text, s-expression). Three approaches:

1. **`nom` parser combinators.** Idiomatic Rust, composable, good
   for grammar-driven inputs. Generates substantial monomorphized
   code; trace-level errors require explicit `context()` annotations
   throughout.
2. **`pest` PEG grammars.** Grammar-as-data; clean separation of
   syntax from semantics. Pegs aren't a great fit for binary
   formats with modal state (OASIS) or formats with recursive
   group nesting (Liberty).
3. **Hand-rolled tokenizer + recursive-descent parser per format.**
   ~2-3× the LOC vs `nom`/`pest`, but: full control over error
   messages (file:line + expected/got), zero macro expansion to
   reason about, easy to skip unknown blocks tolerantly, and the
   binary formats (GDS, OASIS) need explicit modal-variable state
   that doesn't fit a parser-combinator paradigm anyway.

## Decision

Every parser in the workspace is hand-rolled: a tokenizer (state
machine over `&[u8]`) and a recursive-descent parser that consumes
the token stream. Errors are typed via `thiserror` and carry
`(file:line, expected, got)` context.

## Consequences

**Pro:**
- Errors are useful (e.g. `LefError::Expected { line: 47,
  expected: "word", got: "Number(3.0) (wanted MACRO)" }`).
- Unknown sections are skipped tolerantly (`skip_paren_body`,
  `skip_until_semicolon`) — important for forward-compatibility
  with format extensions we haven't seen yet.
- Binary formats (GDS, OASIS) sit alongside text formats with
  identical error-handling ergonomics.
- No `nom` / `pest` upstream version-skew exposure.

**Con:**
- ~2-3× more LOC per parser than the `nom` equivalent.
- Each parser maintains its own state-machine; small changes to
  grammar require touching imperative code rather than a
  declarative grammar file.
- Fuzz coverage matters more here than it would with a
  parser-combinator library — see [`fuzz/`](../../fuzz/) for the
  libfuzzer harnesses.

## Revisit if

- We need to add a 6th text format and the maintenance burden of
  hand-rolled parsers becomes the bottleneck.
- Fuzzing surfaces a class of bugs that `nom`'s combinator-level
  invariants would have caught for free.
