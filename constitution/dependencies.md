# Dependency Rules

Rules governing external crate dependencies, versioning, and allowed dependency directions.

## External Dependencies

All dependencies must be justified by a documented requirement and approved during the release cycle.

- **Rule:** No new external crate may be added to `kernel/Cargo.toml` without a corresponding entry in an ADR or architecture decision document.

- **Rule:** Pin major versions of all dependencies. Do not use `*` or wide range version specifiers.

- **Rule:** The `metrics` and `metrics-exporter-prometheus` crates are the sole approved telemetry libraries. Do not add alternative metrics or tracing exporters.

## Dependency Direction

- **Rule:** The kernel crate must not depend on any domain worker crate. All kernel-to-worker communication flows through the EventBus and process-level invocation.

- **Rule:** The `sdk/` crate may depend on types exported by the kernel, but the kernel must not depend on the `sdk/` crate.

- **Rule:** Domain worker crates may depend on the `sdk/` crate. They must not depend directly on kernel internal modules.
