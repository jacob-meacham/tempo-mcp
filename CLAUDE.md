# Tempo MCP

Lightweight, in-memory calendar MCP server in Rust for LLM scheduling workflows.

## Engineering Standards

Follow the engineering constitution in `agent-instructions/coding/constitution.md`, adapted for Rust:

- **Simplicity over cleverness** — write obvious code; three clear lines beat one clever line
- **Type safety** — use enums, newtypes (`EventId(Uuid)`), and `Option`/`Result` to make illegal states unrepresentable; avoid `unwrap()` in non-test code
- **Explicit error handling** — use `Result<T, TempoError>` everywhere; never silently swallow errors; use `thiserror` for error definitions
- **Single responsibility** — functions < 50 lines, files < 500 lines, modules are cohesive
- **Immutability by default** — prefer `&self` over `&mut self` where possible; clone only when necessary
- **Test everything that matters** — unit tests for all public functions, Given-When-Then structure, descriptive test names

## Project Conventions

- All internal times are UTC (`DateTime<Utc>`); timezone strings preserved for display/export
- MCP transport is stdio — **never** use `println!()` or `print!()`; use `tracing` (writes to stderr)
- Domain logic in `src/calendar/` has zero MCP awareness
- Tool parameter structs derive `Deserialize + JsonSchema` (schemars)
- Error types convert to MCP `ErrorData` via `From` impl

## Architecture

See `docs/design.md` for the full design document.

## Build & Test

```sh
cargo build          # compile
cargo test           # run all tests
cargo clippy         # lint
```
