use crate::models::note::{NoteMeta, NoteStatus, SearchResult};
use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Priority scoring settings — configurable weights for search result ranking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrioritySettings {
    /// Weight for how recently a note was updated (0.0-1.0)
    pub recency_weight: f32,
    /// Half-life for recency decay in days (older notes decay more)
    pub recency_half_life_days: f32,
    /// Score multipliers by note status
    pub content_type_weights: ContentTypeWeights,
    /// Per-tag boost multipliers (tag name → multiplier)
    #[serde(default)]
    pub tag_boosts: HashMap<String, f32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentTypeWeights {
    pub canonical: f32,
    pub evidence: f32,
    pub draft: f32,
}

impl Default for PrioritySettings {
    fn default() -> Self {
        Self {
            recency_weight: 0.3,
            recency_half_life_days: 30.0,
            content_type_weights: ContentTypeWeights {
                canonical: 1.5,
                evidence: 1.0,
                draft: 0.8,
            },
            tag_boosts: HashMap::new(),
        }
    }
}

/// Priority scoring service — re-ranks search results using configurable weights
pub struct PriorityScoringService {
    settings: PrioritySettings,
    settings_path: PathBuf,
}

impl PriorityScoringService {
    pub fn new(data_path: PathBuf) -> Self {
        let settings_path = data_path.join("priority_settings.json");
        let settings = Self::load_from_disk(&settings_path).unwrap_or_default();
        Self {
            settings,
            settings_path,
        }
    }

    fn load_from_disk(path: &PathBuf) -> Result<PrioritySettings> {
        let data = std::fs::read_to_string(path)?;
        let settings: PrioritySettings = serde_json::from_str(&data)?;
        Ok(settings)
    }

    fn save_to_disk(&self) -> Result<()> {
        let data = serde_json::to_string_pretty(&self.settings)?;
        if let Some(parent) = self.settings_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&self.settings_path, data)?;
        Ok(())
    }

    pub fn get_settings(&self) -> &PrioritySettings {
        &self.settings
    }

    pub fn update_settings(&mut self, update: PrioritySettingsUpdate) -> Result<PrioritySettings> {
        if let Some(v) = update.recency_weight {
            self.settings.recency_weight = v.clamp(0.0, 1.0);
        }
        if let Some(v) = update.recency_half_life_days {
            self.settings.recency_half_life_days = v.max(1.0);
        }
        if let Some(v) = update.content_type_weights {
            self.settings.content_type_weights = v;
        }
        if let Some(v) = update.tag_boosts {
            self.settings.tag_boosts = v;
        }
        self.save_to_disk()?;
        Ok(self.settings.clone())
    }

    pub fn reset_settings(&mut self) -> Result<PrioritySettings> {
        self.settings = PrioritySettings::default();
        self.save_to_disk()?;
        Ok(self.settings.clone())
    }

    /// Re-rank search results by applying priority scoring on top of BM25 scores.
    ///
    /// Final score = bm25_score * (1.0 + recency_boost + status_boost + tag_boost)
    pub fn score_results(&self, results: &mut Vec<SearchResult>) {
        let now = Utc::now();

        for result in results.iter_mut() {
            let mut boost = 0.0_f32;

            // Recency boost: exponential decay based on updated_at
            let age_days = (now - result.note.updated_at).num_hours() as f32 / 24.0;
            let decay = (-age_days * 0.693 / self.settings.recency_half_life_days).exp();
            boost += self.settings.recency_weight * decay;

            // Status boost
            let status_weight = match result.note.status {
                NoteStatus::Canonical => self.settings.content_type_weights.canonical,
                NoteStatus::Evidence => self.settings.content_type_weights.evidence,
                NoteStatus::Draft => self.settings.content_type_weights.draft,
            };
            boost += status_weight - 1.0; // Subtract 1.0 so default status doesn't inflate

            // Tag boosts
            for tag in &result.note.tags {
                if let Some(&tag_boost) = self.settings.tag_boosts.get(tag) {
                    boost += tag_boost;
                }
            }

            result.score *= 1.0 + boost;
        }

        // Re-sort by final score descending
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Compute a priority boost for a single note (for chunk-level retrieval).
    pub fn compute_boost(&self, note: &NoteMeta) -> f32 {
        let now = Utc::now();
        let mut boost = 0.0_f32;

        // Recency boost
        let age_days = (now - note.updated_at).num_hours() as f32 / 24.0;
        let decay = (-age_days * 0.693 / self.settings.recency_half_life_days).exp();
        boost += self.settings.recency_weight * decay;

        // Status boost
        let status_weight = match note.status {
            NoteStatus::Canonical => self.settings.content_type_weights.canonical,
            NoteStatus::Evidence => self.settings.content_type_weights.evidence,
            NoteStatus::Draft => self.settings.content_type_weights.draft,
        };
        boost += status_weight - 1.0;

        // Tag boosts
        for tag in &note.tags {
            if let Some(&tag_boost) = self.settings.tag_boosts.get(tag) {
                boost += tag_boost;
            }
        }

        boost
    }
}

/// Partial update for priority settings
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PrioritySettingsUpdate {
    pub recency_weight: Option<f32>,
    pub recency_half_life_days: Option<f32>,
    pub content_type_weights: Option<ContentTypeWeights>,
    pub tag_boosts: Option<HashMap<String, f32>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::NoteMeta;
    use chrono::{Duration, Utc};

    fn make_result(
        id: &str,
        status: NoteStatus,
        age_days: i64,
        tags: Vec<&str>,
        score: f32,
    ) -> SearchResult {
        SearchResult {
            note: NoteMeta {
                id: id.to_string(),
                title: id.to_string(),
                status,
                tags: tags.into_iter().map(String::from).collect(),
                created_at: Utc::now() - Duration::days(age_days),
                updated_at: Utc::now() - Duration::days(age_days),
            },
            score,
            snippet: None,
        }
    }

    #[test]
    fn test_default_settings() {
        let s = PrioritySettings::default();
        assert_eq!(s.recency_weight, 0.3);
        assert_eq!(s.content_type_weights.canonical, 1.5);
        assert!(s.tag_boosts.is_empty());
    }

    #[test]
    fn test_recency_boosts_recent_notes() {
        let svc = PriorityScoringService {
            settings: PrioritySettings::default(),
            settings_path: PathBuf::from("/tmp/test_priority.json"),
        };

        let mut results = vec![
            make_result("old", NoteStatus::Draft, 90, vec![], 1.0),
            make_result("new", NoteStatus::Draft, 1, vec![], 1.0),
        ];

        svc.score_results(&mut results);

        // Recent note should rank higher
        assert_eq!(results[0].note.id, "new");
        assert!(results[0].score > results[1].score);
    }

    #[test]
    fn test_status_boosts_canonical() {
        let svc = PriorityScoringService {
            settings: PrioritySettings::default(),
            settings_path: PathBuf::from("/tmp/test_priority.json"),
        };

        let mut results = vec![
            make_result("draft", NoteStatus::Draft, 1, vec![], 1.0),
            make_result("canon", NoteStatus::Canonical, 1, vec![], 1.0),
        ];

        svc.score_results(&mut results);

        // Canonical note should rank higher
        assert_eq!(results[0].note.id, "canon");
    }

    #[test]
    fn test_tag_boost() {
        let mut tag_boosts = HashMap::new();
        tag_boosts.insert("important".to_string(), 0.5);

        let svc = PriorityScoringService {
            settings: PrioritySettings {
                tag_boosts,
                ..Default::default()
            },
            settings_path: PathBuf::from("/tmp/test_priority.json"),
        };

        let mut results = vec![
            make_result("no-tag", NoteStatus::Draft, 1, vec![], 1.0),
            make_result("tagged", NoteStatus::Draft, 1, vec!["important"], 1.0),
        ];

        svc.score_results(&mut results);

        assert_eq!(results[0].note.id, "tagged");
        assert!(results[0].score > results[1].score);
    }
}
