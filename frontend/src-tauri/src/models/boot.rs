use serde::{Deserialize, Serialize};

/// High-level application startup status exposed to the frontend splash screen.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BootStatus {
    pub phase: String,
    pub message: String,
    pub ready: bool,
    pub error: Option<String>,
}

impl BootStatus {
    pub fn new(phase: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            message: message.into(),
            ready: false,
            error: None,
        }
    }

    pub fn ready(message: impl Into<String>) -> Self {
        Self {
            phase: "ready".to_string(),
            message: message.into(),
            ready: true,
            error: None,
        }
    }

    pub fn failed(phase: impl Into<String>, message: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            phase: phase.into(),
            message: message.into(),
            ready: false,
            error: Some(error.into()),
        }
    }
}

impl Default for BootStatus {
    fn default() -> Self {
        Self::new("starting", "Preparing workspace")
    }
}

#[cfg(test)]
mod tests {
    use super::BootStatus;

    #[test]
    fn ready_status_sets_ready_flag() {
        let status = BootStatus::ready("Ready");
        assert!(status.ready);
        assert_eq!(status.phase, "ready");
        assert_eq!(status.error, None);
    }

    #[test]
    fn failed_status_preserves_error() {
        let status = BootStatus::failed("building_indices", "Startup failed", "disk error");
        assert!(!status.ready);
        assert_eq!(status.phase, "building_indices");
        assert_eq!(status.error.as_deref(), Some("disk error"));
    }
}
