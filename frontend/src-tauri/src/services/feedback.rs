//! Feedback service for submitting bug reports and feature requests via Cloudflare Worker proxy.
//!
//! The worker holds the GitHub token server-side. The desktop app sends feedback
//! to the worker with an anti-abuse key, and the worker creates the GitHub issue.

use crate::models::feedback::{
    FeedbackCreate, FeedbackResponse, FeedbackStatus, FeedbackType, PendingFeedback, SystemInfo,
};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Worker base URL for the Grafyn updater/feedback proxy.
const WORKER_URL: &str = "https://grafyn-updater.grafyn-updater.workers.dev";

/// Anti-abuse key sent via X-Feedback-Key header.
/// This is NOT a secret — it only prevents casual abuse of the feedback endpoint.
/// The worst case if extracted is spam issues on a public repo.
const FEEDBACK_KEY: &str = "gfyn-fb-a7x9k2m4";

/// Service for handling feedback submission via Cloudflare Worker proxy
#[derive(Debug, Clone)]
pub struct FeedbackService {
    client: Client,
    store_path: PathBuf,
}

impl FeedbackService {
    /// Create a new feedback service. No environment variables needed.
    pub fn new(store_path: PathBuf) -> Self {
        // Create the pending feedback directory
        std::fs::create_dir_all(&store_path).ok();

        Self {
            client: Client::new(),
            store_path,
        }
    }

    /// Get the current status of the feedback service
    pub fn get_status(&self) -> FeedbackStatus {
        let pending = self.get_pending().unwrap_or_default();

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

    /// Submit feedback - sends to worker proxy or queues if offline
    pub async fn submit(&self, mut feedback: FeedbackCreate) -> Result<FeedbackResponse> {
        // Validate input
        feedback.validate().map_err(|e| anyhow::anyhow!(e))?;

        // Add system info if requested
        if feedback.include_system_info && feedback.system_info.is_none() {
            feedback.system_info = Some(self.get_system_info(None));
        }

        // Check connectivity
        if !self.is_online().await {
            self.queue_feedback(feedback)?;
            return Ok(FeedbackResponse::queued());
        }

        // Submit to worker
        match self.submit_to_worker(&feedback).await {
            Ok(response) => Ok(response),
            Err(e) => {
                // Queue on failure
                log::warn!("Failed to submit feedback, queueing: {}", e);
                self.queue_feedback(feedback)?;
                Ok(FeedbackResponse::queued())
            }
        }
    }

    /// Check if we can reach the worker
    async fn is_online(&self) -> bool {
        self.client
            .get(WORKER_URL)
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Submit feedback to the Cloudflare Worker proxy
    async fn submit_to_worker(&self, feedback: &FeedbackCreate) -> Result<FeedbackResponse> {
        // Build issue body
        let body = self.format_issue_body(feedback);

        // Determine labels based on feedback type
        let labels = match feedback.feedback_type {
            FeedbackType::Bug => vec!["bug", "user-feedback"],
            FeedbackType::Feature => vec!["enhancement", "user-feedback"],
            FeedbackType::General => vec!["feedback", "user-feedback"],
        };

        let request_body = WorkerFeedbackRequest {
            title: feedback.title.clone(),
            body,
            labels,
        };

        let response = self
            .client
            .post(format!("{}/feedback", WORKER_URL))
            .header("X-Feedback-Key", FEEDBACK_KEY)
            .json(&request_body)
            .send()
            .await
            .context("Failed to send request to feedback proxy")?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!("Feedback proxy error: {}", error_text));
        }

        let issue: WorkerFeedbackResponse = response
            .json()
            .await
            .context("Failed to parse feedback proxy response")?;

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

        body.push_str("\n---\n*Submitted via Grafyn Desktop App*");

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
        if !self.is_online().await {
            return Ok(vec![]);
        }

        let pending = self.get_pending()?;
        let mut results = Vec::new();

        for mut item in pending {
            match self.submit_to_worker(&item.feedback).await {
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
        // Validate ID to prevent path traversal
        if id.is_empty() || id.contains('/') || id.contains('\\') || id.contains("..") {
            anyhow::bail!("Invalid feedback ID: {}", id);
        }
        let file_path = self.store_path.join(format!("{}.json", id));
        if file_path.exists() {
            std::fs::remove_file(&file_path).context("Failed to remove pending feedback file")?;
        }
        Ok(())
    }
}

// Worker proxy request/response types

#[derive(Debug, Serialize)]
struct WorkerFeedbackRequest {
    title: String,
    body: String,
    labels: Vec<&'static str>,
}

#[derive(Debug, Deserialize)]
struct WorkerFeedbackResponse {
    number: u64,
    html_url: String,
}
