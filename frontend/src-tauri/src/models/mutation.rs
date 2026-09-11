use serde::{Deserialize, Serialize};

pub const COMMITTED_WARNING_EVENT: &str = "grafyn://committed-warning";

/// Public, deliberately non-diagnostic notice for a mutation whose authority
/// bytes committed but whose derived views could not be made ready. Internal
/// errors are logged separately and never copied into this DTO.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CommittedMutationWarningV1 {
    pub code: String,
    pub message: String,
}

impl CommittedMutationWarningV1 {
    pub fn derived_state_unavailable() -> Self {
        Self {
            code: "derived_state_unavailable".to_string(),
            message: "Your change was saved, but derived views are temporarily unavailable."
                .to_string(),
        }
    }

    pub fn optimizer_publication_pending() -> Self {
        Self {
            code: "optimizer_publication_pending".to_string(),
            message: "Your change was saved, but its optimizer audit publication is still pending."
                .to_string(),
        }
    }

    pub fn optimizer_rollback_recovery_pending() -> Self {
        Self {
            code: "optimizer_rollback_recovery_pending".to_string(),
            message:
                "The rollback is accepted, but restoring the target bytes is still pending. Do not retry."
                    .to_string(),
        }
    }

    pub fn optimizer_rollback_not_applied() -> Self {
        Self {
            code: "optimizer_rollback_not_applied".to_string(),
            message: "The rollback did not restore target bytes after authority advanced."
                .to_string(),
        }
    }
}

impl std::fmt::Display for CommittedMutationWarningV1 {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn committed_warning_has_fixed_sanitized_wire_shape() {
        assert_eq!(
            serde_json::to_value(CommittedMutationWarningV1::derived_state_unavailable()).unwrap(),
            serde_json::json!({
                "code": "derived_state_unavailable",
                "message": "Your change was saved, but derived views are temporarily unavailable."
            })
        );
        assert_eq!(
            serde_json::to_value(CommittedMutationWarningV1::optimizer_publication_pending())
                .unwrap(),
            serde_json::json!({
                "code": "optimizer_publication_pending",
                "message": "Your change was saved, but its optimizer audit publication is still pending."
            })
        );
        assert_eq!(
            serde_json::to_value(CommittedMutationWarningV1::optimizer_rollback_not_applied())
                .unwrap(),
            serde_json::json!({
                "code": "optimizer_rollback_not_applied",
                "message": "The rollback did not restore target bytes after authority advanced."
            })
        );
        assert_eq!(
            serde_json::to_value(CommittedMutationWarningV1::optimizer_rollback_recovery_pending())
                .unwrap(),
            serde_json::json!({
                "code": "optimizer_rollback_recovery_pending",
                "message": "The rollback is accepted, but restoring the target bytes is still pending. Do not retry."
            })
        );
    }
}
