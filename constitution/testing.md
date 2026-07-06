# Testing Rules

Minimum testing requirements and mandatory test types for code changes.

## Coverage Thresholds

- **Rule:** All new kernel modules must have unit test coverage for at least 80% of public functions and 60% of private functions by line count.

- **Rule:** No kernel test module may be removed or disabled without a documented exception approved by the architecture guardian.

- **Rule:** Each state machine transition must be covered by at least one integration test that exercises the full objective lifecycle.

## Mandatory Test Types

- **Rule:** Every error path in `Result`-returning functions in the kernel must have a corresponding test that triggers the error.

- **Rule:** Any change to the Event Bus event schema must include a round-trip test (publish → subscribe → assert).

- **Rule:** Integration tests must use an in-memory SQLite pool (`SqlitePool::connect("sqlite::memory:")`) rather than a real database file.

## Test Hygiene

- **Rule:** Tests must not depend on wall-clock timing. Use `tokio::time::pause()` or deterministic clock mocks instead of `sleep()`.

- **Rule:** Test files must be in a `tests` module annotated with `#[cfg(test)]` at the bottom of the source file, not in a separate `tests/` directory, for unit tests.
