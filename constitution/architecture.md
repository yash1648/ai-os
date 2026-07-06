# Architecture Rules

Rules governing module boundaries, dependency direction, and system architecture.

## Module Isolation

Each kernel module must declare its public interface via a `pub mod` or re-export. Internal implementation details must remain private (`pub(crate)` or module-private).

- **Rule:** Kernel modules must not circularly depend on each other. If module A calls module B, B must not directly call A — use the EventBus for inversion.

- **Rule:** The API layer (`api.rs`) is the only module that may directly expose HTTP routes. Internal modules must not reference axum or tower types.

- **Rule:** Each domain worker crate must declare its interface in a `lib.rs` that re-exports only the public API surface.

## Hexagonal Architecture

The system follows a layered architecture where outer layers depend on inner layers, never the reverse.

- **Rule:** The EventBus is the sole mechanism for cross-module communication during objective lifecycle transitions. Modules must not call each other's internal functions for lifecycle coordination.

- **Rule:** Persistence (SQLite via sqlx) must be abstracted behind a repository or data-access module. Raw SQL must not appear in API handlers or business logic outside `*_store` or `*_repo` modules.
