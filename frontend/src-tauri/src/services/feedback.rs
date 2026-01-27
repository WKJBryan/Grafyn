//! Feedback service for submitting bug reports and feature requests to GitHub

use crate::models::feedback::{
    FeedbackCreate, FeedbackResponse, FeedbackStatus, FeedbackType, PendingFeedback, SystemInfo,
};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

const GITHUB_API_URL: &str = "https://api.github.com";

/// Service for handling feedback submission to GitHub Issues
#[derive(Debug, Clone)]
pub struct FeedbackService {
    client: Client,
    store_path: PathBuf,
    repo: String,
    token: String,
}

impl FeedbackService {
    /// Create a new feedback service with environment variables (for development)
    pub fn new(store_path: PathBuf) -> Self {
        // Create the pending feedback directory
        std::fs::create_dir_all(&store_path).ok();

        // Load configuration from environment
        let repo = std::env::var("GITHUB_FEEDBACK_REPO").unwrap_or_default();
        let token = std::env::var("GITHUB_FEEDBACK_TOKEN").unwrap_or_default();

        Self {
            client: Client::new(),
            store_path,
            repo,
            token,
        }
    }

    /// Create a new feedback service with explicit credentials (for production)
    /// Use this to embed credentials at compile time
    pub fn new_with_credentials(store_path: PathBuf, repo: String, token: String) -> Self {
        // Create the pending feedback directory
        std::fs::create_dir_all(&store_path).ok();

        Self {
            client: Client::new(),
            store_path,
            repo,
            token,
        }
    }

    /// Check if the service is properly configured
    pub fn is_configured(&self) -> bool {
        !self.repo.is_empty() && !self.token.is_empty()
    }

    /// Get the current status of the feedback service
    pub fn get_status(&self) -> FeedbackStatus {
        let pending = self.get_pending().unwrap_or_default();

        if !self.is_configured() {
            return FeedbackStatus {
                configured: false,
                pending_count: pending.len(),
                message: "Feedback service not configured. Set GITHUB_FEEDBACK_REPO and GITHUB_FEEDBACK_TOKEN environment variables.".to_string(),
            };
        }

        FeedbackStatus {
            configured: true,
            pending_count: pending.len(),
            message: if pending.is_empty() {
                "Feedback service ready".to_string()
            } else {
                format!("{} feedback item(s) pending submission", pending.len())
            },
        }
    }

    /// Get system information for the current platform
    pub fn get_system_info(&self, current_page: Option<String>) -> SystemInfo {
        let platform = format!(
            "{} {}",
            std::env::consts::OS,
            std::env::consts::ARCH
        );

        SystemInfo {
            platform,
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            runtime: "tauri-desktop".to_string(),
            current_page,
        }
    }

    /// Submit feedback - creates GitHub issue or queues if offline
    pub async fn submit(&self, mut feedback: FeedbackCreate) -> Result<FeedbackResponse> {
        // Validate input
        feedback.validate().map_err(|e| anyhow::anyhow!(e))?;

        // Add system info if requested
        if feedback.include_system_info && feedback.system_info.is_none() {
            feedback.system_info = Some(self.get_system_info(None));
        }

        // Check configuration
        if !self.is_configured() {
            return Ok(FeedbackResponse::error(
                "Feedback service not configured. Please set GITHUB_FEEDBACK_REPO and GITHUB_FEEDBACK_TOKEN.",
            ));
        }

        // Check connectivity
        if !self.is_online().await {
            self.queue_feedback(feedback)?;
            return Ok(FeedbackResponse::queued());
        }

        // Submit to GitHub
        match self.submit_to_github(&feedback).await {
            Ok(response) => Ok(response),
            Err(e) => {
                // Queue on failure
                log::warn!("Failed to submit feedback, queueing: {}", e);
                self.queue_feedback(feedback)?;
                Ok(FeedbackResponse::queued())
            }
        }
    }

    /// Check if we can reach GitHub API
    async fn is_online(&self) -> bool {
        self.client
            .head(format!("{}/rate_limit", GITHUB_API_URL))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "Seedream-Desktop")
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Submit feedback directly to GitHub Issues API
    async fn submit_to_github(&self, feedback: &FeedbackCreate) -> Result<FeedbackResponse> {
        let (owner, repo) = self
            .repo
            .split_once('/')
            .context("Invalid repo format, expected 'owner/repo'")?;

        // Build issue body
        let body = self.format_issue_body(feedback);

        // Determine labels based on feedback type
        let labels = match feedback.feedback_type {
            FeedbackType::Bug => vec!["bug", "user-feedback"],
            FeedbackType::Feature => vec!["enhancement", "user-feedback"],
            FeedbackType::General => vec!["feedback", "user-feedback"],
        };

        let request_body = GitHubIssueCreate {
            title: feedback.title.clone(),
            body,
            labels,
        };

        let response = self
            .client
            .post(format!("{}/repos/{}/{}/issues", GITHUB_API_URL, owner, repo))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("User-Agent", "Seedream-Desktop")
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to GitHub")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("GitHub API error: {}", error_text));
        }

        let issue: GitHubIssueResponse = response
            .json()
            .await
            .context("Failed to parse GitHub response")?;

        Ok(FeedbackResponse::success(issue.number, issue.html_url))
    }

    /// Format the GitHub issue body with feedback details
    fn format_issue_body(&self, feedback: &FeedbackCreate) -> String {
        let type_emoji = match feedback.feedback_type {
            FeedbackType::Bug => "🐛",
            FeedbackType::Feature => "💡",
            FeedbackType::General => "💬",
        };

        let type_label = match feedback.feedback_type {
            FeedbackType::Bug => "Bug Report",
            FeedbackType::Feature => "Feature Request",
            FeedbackType::General => "General Feedback",
        };

        let mut body = format!(
            "## {} {}\n\n{}\n\n",
            type_emoji, type_label, feedback.description
        );

        if let Some(ref system_info) = feedback.system_info {
            body.push_str("---\n\n");
            body.push_str("### System Information\n\n");
            body.push_str(&format!("- **Platform:** {}\n", system_info.platform));
            body.push_str(&format!("- **App Version:** {}\n", system_info.app_version));
            body.push_str(&format!("- **Runtime:** {}\n", system_info.runtime));
            if let Some(ref page) = system_info.current_page {
                body.push_str(&format!("- **Current Page:** {}\n", page));
            }
        }

        body.push_str("\n---\n*Submitted via Seedream Desktop App*");

        body
    }

    /// Queue feedback for later submission (offline mode)
    fn queue_feedback(&self, feedback: FeedbackCreate) -> Result<()> {
        let pending = PendingFeedback::new(feedback);
        let file_path = self.store_path.join(format!("{}.json", pending.id));

        let json = serde_json::to_string_pretty(&pending)
            .context("Failed to serialize pending feedback")?;

        std::fs::write(&file_path, json).context("Failed to write pending feedback file")?;

        log::info!("Queued feedback: {}", pending.id);
        Ok(())
    }

    /// Get all pending feedback items
    pub fn get_pending(&self) -> Result<Vec<PendingFeedback>> {
        let mut pending = Vec::new();

        if !self.store_path.exists() {
            return Ok(pending);
        }

        for entry in std::fs::read_dir(&self.store_path)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(item) = serde_json::from_str::<PendingFeedback>(&content) {
                        pending.push(item);
                    }
                }
            }
        }

        // Sort by creation time, oldest first
        pending.sort_by(|a, b| a.created_at.cmp(&b.created_at));

        Ok(pending)
    }

    /// Retry submitting pending feedback items
    pub async fn retry_pending(&self) -> Result<Vec<FeedbackResponse>> {
        if !self.is_configured() {
            return Ok(vec![]);
        }

        if !self.is_online().await {
            return Ok(vec![]);
        }

        let pending = self.get_pending()?;
        let mut results = Vec::new();

        for mut item in pending {
            match self.submit_to_github(&item.feedback).await {
                Ok(response) => {
                    // Remove from queue on success
                    let file_path = self.store_path.join(format!("{}.json", item.id));
                    std::fs::remove_file(&file_path).ok();
                    log::info!("Successfully submitted queued feedback: {}", item.id);
                    results.push(response);
                }
                Err(e) => {
                    // Update retry count
                    item.retry_count += 1;
                    let file_path = self.store_path.join(format!("{}.json", item.id));
                    if let Ok(json) = serde_json::to_string_pretty(&item) {
                        std::fs::write(&file_path, json).ok();
                    }
                    log::warn!(
                        "Failed to retry feedback {} (attempt {}): {}",
                        item.id,
                        item.retry_count,
                        e
                    );
                    results.push(FeedbackResponse::error(format!(
                        "Failed to submit: {}",
                        e
                    )));
                }
            }
        }

        Ok(results)
    }

    /// Clear a specific pending feedback item
    pub fn clear_pending(&self, id: &str) -> Result<()> {
        let file_path = self.store_path.join(format!("{}.json", id));
        if file_path.exists() {
            std::fs::remove_file(&file_path).context("Failed to remove pending feedback file")?;
        }
        Ok(())
    }
}

// GitHub API types

#[derive(Debug, Serialize)]
struct GitHubIssueCreate {
    title: String,
    body: String,
    labels: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
struct GitHubIssueResponse {
    number: u64,
    html_url: String,
}
