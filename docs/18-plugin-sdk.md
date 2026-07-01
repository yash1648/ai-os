# AI-OS Documentation — 18. Plugin SDK Specification

## Purpose

AI-OS is language- and framework-agnostic at its core; concrete support for a given language or framework is delivered through plugins. This keeps the Kernel, Scheduler, and Guardian free of language-specific logic and lets the community (or an organization) add support for new ecosystems without modifying core components.

## Plugin Categories

### Language Plugins
Provide: parsing (AST extraction feeding the Symbol Graph), linting/style-check integration, test runner integration, build/compile integration, and dependency-manifest parsing (feeding the Dependency Graph).

Initial targets: Java, Python, TypeScript, Rust, Go.

### Framework Plugins
Provide framework-specific conventions on top of a language plugin: route/endpoint detection (for Interface Registry population), framework-idiomatic architecture rule defaults (e.g., a Spring Boot layering convention), and framework-specific test scaffolding.

Initial targets: Spring Boot, FastAPI, React, Next.js.

### Guardian Rule Plugins
Provide custom static-analysis checks beyond the standard dependency/interface rules — e.g., a security-focused taint-analysis plugin, or an organization-specific compliance check.

## Plugin Interface (Language Plugin Example)

```yaml
plugin_id: string
kind: language
targets: [file_extensions]
capabilities:
  parse: fn(source) -> AST
  extract_symbols: fn(AST) -> [Symbol]
  extract_dependencies: fn(AST) -> [DependencyEdge]
  run_tests: fn(workspace, test_selector) -> TestResult
  run_build: fn(workspace) -> BuildResult
  lint: fn(source, config) -> [LintFinding]
```

## Plugin Isolation

Plugins execute in the same sandboxed boundary as Workers — they can be invoked by the PIL (for indexing) or the Guardian (for rule evaluation), but they never gain Kernel-level write authority. A misbehaving or buggy plugin can produce incorrect analysis (a quality problem, escalated via monitoring) but cannot bypass the Kernel's write-mediation guarantee.

## Versioning and Compatibility

Plugins declare a compatibility range against the AI-OS core API version. The Kernel refuses to load a plugin outside its declared compatible range rather than risking undefined behavior from an API mismatch.

## Contribution Path

New plugins are contributed following the standard Contributor Guide (`27-contributor-guide.md`) and, for any plugin that introduces new Guardian rule types, an accompanying RFC (`28-rfc-process.md`) describing the rule's semantics and default severity.
