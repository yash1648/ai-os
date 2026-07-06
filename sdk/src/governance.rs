//! Governance stubs — approval workflows, compliance records, and CSV export.
//!
//! These types are shared between the kernel's governance API and plugin
//! evaluations that require human approval or produce compliance artifacts.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// ApprovalRequest
// ---------------------------------------------------------------------------

/// A request for human approval, typically raised by a plugin when a diff
/// requires manual review before proceeding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// Unique identifier for this approval request.
    pub request_id: String,
    /// The objective that triggered this request.
    pub objective_id: String,
    /// The plugin or system component that requested approval.
    pub requested_by: String,
    /// Role required to approve this request (e.g., `"admin"`, `"lead"`).
    pub required_role: String,
    /// Current status of the approval request.
    pub status: ApprovalStatus,
    /// ISO 8601 timestamp of when this request was created.
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// ApprovalStatus
// ---------------------------------------------------------------------------

/// The status lifecycle of an approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApprovalStatus {
    /// Waiting for a human decision.
    Pending,
    /// Approved with an optional rationale.
    Approved(String),
    /// Rejected with an optional reason.
    Rejected(String),
}

// ---------------------------------------------------------------------------
// ComplianceRecord
// ---------------------------------------------------------------------------

/// A compliance record that captures the full review and approval history
/// for a single objective.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceRecord {
    /// The objective identifier.
    pub objective_id: String,
    /// Human-readable title of the objective.
    pub title: String,
    /// Compliance status (e.g., `"compliant"`, `"non_compliant"`,
    /// `"pending_review"`).
    pub status: String,
    /// Domain scope of this record (e.g., `"quality"`, `"security"`).
    pub domain: String,
    /// ISO 8601 timestamp of when the compliance review started.
    pub created_at: String,
    /// ISO 8601 timestamp of when the compliance review completed,
    /// if applicable.
    pub completed_at: Option<String>,
    /// Verdict issued by the code review step.
    pub review_verdict: Option<String>,
    /// Verdict issued by the guardian step.
    pub guardian_verdict: Option<String>,
}

// ---------------------------------------------------------------------------
// ComplianceExporter
// ---------------------------------------------------------------------------

/// Utility for exporting compliance records in various formats.
///
/// Currently supports CSV export. Additional formats may be added later.
pub struct ComplianceExporter;

impl ComplianceExporter {
    /// Export a slice of compliance records as a CSV string.
    ///
    /// The CSV includes a header row and one row per record.
    pub fn export_csv(&self, records: &[ComplianceRecord]) -> String {
        let mut csv = String::from(
            "objective_id,title,status,domain,created_at,completed_at,review_verdict,guardian_verdict\n",
        );

        for r in records {
            // Escape fields that may contain commas or quotes.
            let title = escape_csv(&r.title);
            let completed_at = r.completed_at.as_deref().unwrap_or("");
            let review_verdict = r.review_verdict.as_deref().unwrap_or("");
            let guardian_verdict = r.guardian_verdict.as_deref().unwrap_or("");

            csv.push_str(&format!(
                "{},{},{},{},{},{},{},{}\n",
                r.objective_id,
                title,
                r.status,
                r.domain,
                r.created_at,
                completed_at,
                review_verdict,
                guardian_verdict,
            ));
        }

        csv
    }
}

/// Escape a string for safe inclusion in a CSV field.
fn escape_csv(value: &str) -> String {
    if value.contains(',') || value.contains('"') || value.contains('\n') {
        let escaped = value.replace('"', "\"\"");
        format!("\"{}\"", escaped)
    } else {
        value.to_string()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn approval_request_pending() {
        let req = ApprovalRequest {
            request_id: "req-1".into(),
            objective_id: "obj-1".into(),
            requested_by: "example-validator".into(),
            required_role: "admin".into(),
            status: ApprovalStatus::Pending,
            created_at: "2025-01-01T00:00:00Z".into(),
        };
        assert!(matches!(req.status, ApprovalStatus::Pending));
        assert_eq!(req.required_role, "admin");
    }

    #[test]
    fn approval_status_serialization() {
        let status = ApprovalStatus::Approved("Looks good".into());
        let json = serde_json::to_string(&status).expect("serialize");
        let back: ApprovalStatus = serde_json::from_str(&json).expect("deserialize");
        assert!(matches!(back, ApprovalStatus::Approved(_)));
    }

    #[test]
    fn compliance_record_roundtrip() {
        let record = ComplianceRecord {
            objective_id: "obj-42".into(),
            title: "Add feature X".into(),
            status: "compliant".into(),
            domain: "quality".into(),
            created_at: "2025-06-01T12:00:00Z".into(),
            completed_at: Some("2025-06-01T14:00:00Z".into()),
            review_verdict: Some("pass".into()),
            guardian_verdict: Some("pass".into()),
        };

        let json = serde_json::to_string(&record).expect("serialize");
        let back: ComplianceRecord = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(back.objective_id, "obj-42");
        assert_eq!(back.status, "compliant");
        assert_eq!(back.completed_at, Some("2025-06-01T14:00:00Z".into()));
    }

    #[test]
    fn compliance_exporter_empty() {
        let exporter = ComplianceExporter;
        let csv = exporter.export_csv(&[]);
        assert_eq!(
            csv,
            "objective_id,title,status,domain,created_at,completed_at,review_verdict,guardian_verdict\n"
        );
    }

    #[test]
    fn compliance_exporter_single_record() {
        let exporter = ComplianceExporter;
        let records = vec![ComplianceRecord {
            objective_id: "obj-1".into(),
            title: "Simple".into(),
            status: "compliant".into(),
            domain: "quality".into(),
            created_at: "2025-01-01T00:00:00Z".into(),
            completed_at: None,
            review_verdict: None,
            guardian_verdict: None,
        }];

        let csv = exporter.export_csv(&records);
        assert!(csv.contains("obj-1"));
        assert!(csv.contains("Simple"));
        assert!(csv.contains("compliant"));
    }

    #[test]
    fn compliance_exporter_escapes_commas() {
        let exporter = ComplianceExporter;
        let records = vec![ComplianceRecord {
            objective_id: "obj-1".into(),
            title: "Title, with, commas".into(),
            status: "compliant".into(),
            domain: "quality".into(),
            created_at: "2025-01-01T00:00:00Z".into(),
            completed_at: None,
            review_verdict: None,
            guardian_verdict: None,
        }];

        let csv = exporter.export_csv(&records);
        // The title should be quoted because it contains commas.
        assert!(csv.contains("\"Title, with, commas\""));
    }
}
