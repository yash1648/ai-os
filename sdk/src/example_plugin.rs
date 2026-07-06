//! Example plugin — demonstrates how to implement the [`Plugin`] trait.
//!
//! This trivial plugin:
//! - Returns [`PluginVerdict::Pass`] for diffs with fewer than 10 changed files.
//! - Returns [`PluginVerdict::Fail`] for diffs with 10 or more changed files.

use crate::{Plugin, PluginContext, PluginResult, PluginVerdict, StructuredDiff};

/// A trivial example plugin that rejects diffs touching 10 or more files.
pub struct ExampleValidator;

impl Plugin for ExampleValidator {
    fn name(&self) -> &str {
        "example-validator"
    }

    fn evaluate(&self, diff: &StructuredDiff, _context: &PluginContext) -> PluginResult {
        let count = diff.files.len();

        if count < 10 {
            PluginResult {
                verdict: PluginVerdict::Pass,
                findings: vec![],
                confidence: 1.0,
            }
        } else {
            PluginResult {
                verdict: PluginVerdict::Fail(format!(
                    "Diff touches {} files; maximum allowed is 9",
                    count
                )),
                findings: vec![],
                confidence: 1.0,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DiffFile, PluginContext};
    use std::collections::HashMap;

    #[test]
    fn example_passes_small_diff() {
        let plugin = ExampleValidator;
        let diff = StructuredDiff {
            files: vec![
                DiffFile {
                    path: "src/main.rs".into(),
                    language: Some("rust".into()),
                    lines_added: 5,
                    lines_removed: 0,
                    diff: "+fn main() {}".into(),
                },
            ],
            metadata: HashMap::new(),
        };
        let context = PluginContext {
            objective_id: "obj-1".into(),
            domain: None,
            config: HashMap::new(),
        };

        let result = plugin.evaluate(&diff, &context);
        assert!(matches!(result.verdict, PluginVerdict::Pass));
    }

    #[test]
    fn example_fails_large_diff() {
        let plugin = ExampleValidator;
        let files: Vec<DiffFile> = (0..10)
            .map(|i| DiffFile {
                path: format!("file-{}.rs", i),
                language: Some("rust".into()),
                lines_added: 1,
                lines_removed: 0,
                diff: "+ ".into(),
            })
            .collect();
        let diff = StructuredDiff {
            files,
            metadata: HashMap::new(),
        };
        let context = PluginContext {
            objective_id: "obj-2".into(),
            domain: None,
            config: HashMap::new(),
        };

        let result = plugin.evaluate(&diff, &context);
        assert!(matches!(result.verdict, PluginVerdict::Fail(_)));
    }
}
