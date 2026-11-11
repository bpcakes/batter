//! The report every command ends with: one JSON document under
//! `--output json`, readable lines under `--output text`.

use serde::Serialize;
use serde_json::Value;

use crate::exit::Exit;

/// How the report is written.
#[derive(Clone, Copy, Debug, Eq, PartialEq, clap::ValueEnum)]
pub(crate) enum Output {
    Text,
    Json,
}

/// The command's report: the experimental scripting contract. Renaming a
/// field is a breaking change.
#[derive(Debug, Serialize)]
pub(crate) struct Report {
    pub(crate) command: &'static str,
    pub(crate) exit: u8,
    pub(crate) outcome: &'static str,
    pub(crate) message: String,
    #[serde(skip_serializing_if = "Value::is_null")]
    pub(crate) data: Value,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub(crate) warnings: Vec<String>,
}

impl Report {
    pub(crate) fn new(
        command: &'static str,
        exit: Exit,
        outcome: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            command,
            exit: exit.code(),
            outcome,
            message: message.into(),
            data: Value::Null,
            warnings: Vec::new(),
        }
    }

    pub(crate) fn with_data(mut self, data: Value) -> Self {
        self.data = data;
        self
    }

    pub(crate) fn warn(mut self, warning: impl Into<String>) -> Self {
        self.warnings.push(warning.into());
        self
    }

    /// The report as written under `output`: always one document for JSON.
    pub(crate) fn render(&self, output: Output) -> String {
        match output {
            Output::Json => serde_json::to_string(self).unwrap_or_else(|_| {
                format!("{{\"command\":\"{}\",\"exit\":1,\"outcome\":\"failed\",\"message\":\"report could not be rendered\"}}", self.command)
            }),
            Output::Text => {
                let mut text = format!("{}: {}", self.command, self.message);
                if !self.data.is_null() {
                    text.push('\n');
                    text.push_str(&serde_json::to_string_pretty(&self.data).unwrap_or_default());
                }
                for warning in &self.warnings {
                    text.push_str("\nwarning: ");
                    text.push_str(warning);
                }
                text
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_is_one_document_with_the_contract_fields() {
        let report = Report::new("set", Exit::Precondition, "refused", "nothing written")
            .with_data(serde_json::json!({"refusal": "undeclared"}))
            .warn("check the fleet");
        let rendered = report.render(Output::Json);
        assert!(!rendered.contains('\n'));
        let parsed: Value = serde_json::from_str(&rendered).unwrap_or(Value::Null);
        assert_eq!(parsed["exit"], 3);
        assert_eq!(parsed["outcome"], "refused");
        assert_eq!(parsed["data"]["refusal"], "undeclared");
        assert_eq!(parsed["warnings"][0], "check the fleet");
        let text = report.render(Output::Text);
        assert!(text.starts_with("set: nothing written"));
        assert!(text.contains("warning: check the fleet"));
    }
}
