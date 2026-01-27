//! Feedback data models for bug reports and feature requests

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Type of feedback being submitted
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum FeedbackType {
    Bug,
    Feature,
    #[default]
    General,
}

impl std::fmt::Display for FeedbackType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedbackType::Bug => write!(f, "bug"),
            FeedbackType::Feature => write!(f, "feature"),
            FeedbackType::General => write!(f, "general"),
        }
    }
}

impl std::str::FromStr for FeedbackType {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "bug" => Ok(FeedbackType::Bug),
            "feature" => Ok(FeedbackType::Feature),
            "general" => Ok(FeedbackType::General),
            _ => Err(format!("Unknown feedback type: {}", s)),
        }
    }
}

/// System information collected with feedback (opt-in)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    /// Operating system name and version
    pub platform: String,
    /// Application version from Cargo.toml
    pub app_version: String,
    /// Runtime environment (Tauri desktop)
    pub runtime: String,
    /// Current page/view in the app
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_page: Option<String>,
}

impl Default for SystemInfo {
    fn default() -> Self {
        Self {
            platform: String::new(),
            app_version: String::new(),
            runtime: "tauri-desktop".to_string(),
            current_page: None,
        }
    }
}

/// Request to create new feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackCreate {
    /// Short summary of the feedback (5-200 chars)
    pub title: String,
    /// Detailed description (10-10000 chars)
    pub description: String,
    /// Type of feedback
    #[serde(default)]
    pub feedback_type: FeedbackType,
    /// Whether to include system information
    #[serde(default)]
    pub include_system_info: bool,
    /// System information (populated if include_system_info is true)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_info: Option<SystemInfo>,
}

impl FeedbackCreate {
    /// Validate the feedback data
    pub fn validate(&self) -> Result<(), String> {
        let title_len = self.title.trim().len();
        if title_len < 5 {
            return Err("Title must be at least 5 characters".to_string());
        }
        if title_len > 200 {
            return Err("Title must be at most 200 characters".to_string());
        }

        let desc_len = self.description.trim().len();
        if desc_len < 10 {
            return Err("Description must be at least 10 characters".to_string());
        }
        if desc_len > 10000 {
            return Err("Description must be at most 10000 characters".to_string());
        }

        Ok(())
    }
}

/// Response after submitting feedback
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackResponse {
    /// Whether submission was successful
    pub success: bool,
    /// GitHub issue number (if created)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_number: Option<u64>,
    /// GitHub issue URL (if created)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issue_url: Option<String>,
    /// User-friendly message
    pub message: String,
    /// Whether feedback was queued for later submission (offline mode)
    #[serde(default)]
    pub queued: bool,
}

impl FeedbackResponse {
    /// Create a success response with GitHub issue details
    pub fn success(issue_number: u64, issue_url: String) -> Self {
        Self {
            success: true,
            issue_number: Some(issue_number),
            issue_url: Some(issue_url),
            message: format!("Feedback submitted successfully as issue #{}", issue_number),
            queued: false,
        }
    }

    /// Create a queued response for offline mode
    pub fn queued() -> Self {
        Self {
            success: true,
            issue_number: None,
            issue_url: None,
            message: "Feedback queued for later submission".to_string(),
            queued: true,
        }
    }

    /// Create an error response
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            success: false,
            issue_number: None,
            issue_url: None,
            message: message.into(),
            queued: false,
        }
    }
}

/// Pending feedback stored locally for offline mode
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingFeedback {
    /// Unique identifier
    pub id: String,
    /// The feedback data
    pub feedback: FeedbackCreate,
    /// When it was created
    pub created_at: DateTime<Utc>,
    /// Number of retry attempts
    #[serde(default)]
    pub retry_count: u32,
}

impl PendingFeedback {
    /// Create new pending feedback with a random ID
    pub fn new(feedback: FeedbackCreate) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            feedback,
            created_at: Utc::now(),
            retry_count: 0,
        }
    }
}

/// Status of the feedback service
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FeedbackStatus {
    /// Whether the service is properly configured
    pub configured: bool,
    /// Number of pending feedback items (offline queue)
    pub pending_count: usize,
    /// User-friendly status message
    pub message: String,
}
