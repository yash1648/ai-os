# AI-OS Documentation — 20. JSON Schemas Reference

This document catalogs the canonical JSON Schema definitions for every core AI-OS object. Full machine-readable schema files live under `schemas/` in the repository; this page summarizes each object's shape and validation rules.

## Objective

```json
{
  "$id": "https://ai-os.dev/schemas/objective.json",
  "type": "object",
  "required": ["id", "title", "owner", "priority", "status", "success_criteria"],
  "properties": {
    "id": {"type": "string"},
    "title": {"type": "string"},
    "owner": {"type": "string"},
    "priority": {"enum": ["low", "medium", "high", "critical"]},
    "dependencies": {"type": "array", "items": {"type": "string"}},
    "status": {"enum": ["DISCOVERED","PLANNED","READY","EXECUTING","REVIEW","INTEGRATION","DONE","PLANNING_FAILURE","PERMISSION_FAILURE","EXECUTION_FAILURE","REVIEW_FAILURE","INTEGRATION_FAILURE","HUMAN_REJECTED","ROLLBACK","ABANDONED"]},
    "success_criteria": {"type": "array", "items": {"type": "string"}, "minItems": 1}
  }
}
```

## Execution Manifest

See `09-execution-manifest.md` for the annotated YAML; the JSON Schema mirrors it 1:1, with `allowed_files` required to be non-empty and `output_schema` required to reference a registered schema version.

## Interface Registry Entry

See `10-interface-registry.md`. Validation rule of note: `compatibility.breaking_change_policy` is required; `sunset_date` is required if `deprecated_since` is set.

## ADR

See `12-adr-system.md`. Validation rule of note: `status: superseded` requires a non-null `superseded_by`; `status: proposed` forbids `superseded_by`.

## Event

See `08-event-bus.md`. Validation rule of note: every event requires `objective_id` except plan-level events (`PlanGenerated`, `PlanApproved`), which require `plan_id` instead.

## Worker Output (Diff + Report)

```json
{
  "$id": "https://ai-os.dev/schemas/worker-output.json",
  "type": "object",
  "required": ["diff", "report"],
  "properties": {
    "diff": {"type": "string"},
    "report": {
      "type": "object",
      "required": ["summary", "files_changed", "confidence"],
      "properties": {
        "summary": {"type": "string"},
        "files_changed": {"type": "array", "items": {"type": "string"}},
        "tests_added": {"type": "array", "items": {"type": "string"}},
        "assumptions": {"type": "array", "items": {"type": "string"}},
        "confidence": {"enum": ["low", "medium", "high"]},
        "open_questions": {"type": "array", "items": {"type": "string"}}
      }
    }
  }
}
```

## Reviewer / Guardian Verdicts

See `16-review-pipeline.md` and `17-architecture-guardian.md` for annotated shapes; both share a common base schema (`verdict`, plus a category-specific findings/violations array).

## Schema Governance

All schemas are versioned independently. A schema change that removes a required field or narrows an enum is treated as a breaking change and follows the same Interface Registry compatibility policy as any other project interface — including, where applicable, a mandatory Human Approval Gate.
