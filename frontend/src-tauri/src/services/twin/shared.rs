#[cfg(test)]
use super::TwinStore;
#[cfg(test)]
use crate::models::twin::{
    ActionGapCreate, ConstitutionItemCreate, ConstitutionStatus, DecisionEpisodeCreate,
    PrimitiveDecisionAssessment, RecordOrigin, ReflectionCardCreate, UserRecord,
};
use crate::models::twin::{TraceEvent, TraceEventType};
#[cfg(test)]
use chrono::Utc;
use serde::de::DeserializeOwned;
#[cfg(test)]
use serde_json::json;
use serde_json::Value;
#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

const EXCERPT_MAX_CHARS: usize = 220;

/// Outcome of attempting to load a single JSON file within a per-file directory listing.
pub(super) enum FileLoadOutcome<T> {
    Loaded(T),
    /// The file couldn't be read (permissions, transient lock, etc). This is not
    /// necessarily corruption — the file may be fine — so it must not be quarantined.
    ReadFailed(anyhow::Error),
    /// The file was read successfully but its content didn't parse as JSON. This is
    /// genuine corruption (truncated write, hand-edit gone wrong) and should be
    /// quarantined so it never bricks the listing again.
    ParseFailed(anyhow::Error),
}

pub(super) fn load_json_file<T: DeserializeOwned>(path: &Path) -> FileLoadOutcome<T> {
    let content = match std::fs::read_to_string(path) {
        Ok(content) => content,
        Err(e) => return FileLoadOutcome::ReadFailed(anyhow::Error::new(e)),
    };
    match serde_json::from_str::<T>(&content) {
        Ok(value) => FileLoadOutcome::Loaded(value),
        Err(e) => FileLoadOutcome::ParseFailed(anyhow::Error::new(e)),
    }
}

/// Rename a corrupt file to `{name}.corrupt-{unix-timestamp}` in the same directory so
/// the bytes aren't lost. Best-effort: if the rename itself fails, log and leave the
/// file in place (it's still skipped for this pass by the caller).
fn quarantine_corrupt_file(path: &Path) {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
        log::error!(
            "Cannot quarantine file with non-UTF8 name: {}",
            path.display()
        );
        return;
    };
    let quarantine_path = path.with_file_name(format!("{file_name}.corrupt-{timestamp}"));

    match std::fs::rename(path, &quarantine_path) {
        Ok(()) => log::error!(
            "Quarantined corrupt file {} to {}",
            path.display(),
            quarantine_path.display()
        ),
        Err(e) => log::error!(
            "Failed to quarantine corrupt file {} to {}: {}",
            path.display(),
            quarantine_path.display(),
            e
        ),
    }
}

/// Load a single JSON file within a per-file directory-listing loop (record cache fill,
/// session-trace listing, decision episodes, constitution items, action gaps, reflection
/// cards). Instead of `?`-propagating and letting one bad file brick the whole listing,
/// this skips the offending file and returns `None`:
/// - a parse failure quarantines the file (renamed + logged) so it's never retried, and
/// - a read failure is skipped without quarantine, since it may be transient/permissions
///   and the file itself could be perfectly fine.
pub(super) fn load_or_quarantine<T: DeserializeOwned>(path: &Path, kind: &str) -> Option<T> {
    match load_json_file::<T>(path) {
        FileLoadOutcome::Loaded(value) => Some(value),
        FileLoadOutcome::ReadFailed(e) => {
            log::error!(
                "Skipping unreadable {} file {}: {}",
                kind,
                path.display(),
                e
            );
            None
        }
        FileLoadOutcome::ParseFailed(e) => {
            log::error!(
                "Skipping corrupt {} file {}: {} — quarantining",
                kind,
                path.display(),
                e
            );
            quarantine_corrupt_file(path);
            None
        }
    }
}

pub(super) fn lexical_terms(text: &str) -> HashSet<String> {
    const STOPWORDS: &[&str] = &[
        "a", "an", "and", "are", "as", "at", "be", "by", "can", "do", "for", "from", "how", "i",
        "if", "in", "into", "is", "it", "its", "me", "my", "of", "on", "or", "our", "so", "that",
        "the", "their", "them", "this", "to", "up", "use", "was", "we", "what", "when", "where",
        "which", "who", "why", "with", "would", "you", "your",
    ];

    text.split(|ch: char| !ch.is_alphanumeric())
        .filter_map(|token| {
            let token = token.trim().to_lowercase();
            if token.len() < 3 || STOPWORDS.contains(&token.as_str()) {
                None
            } else {
                Some(token)
            }
        })
        .collect()
}

pub(super) fn evidence_note(event: &TraceEvent) -> Option<String> {
    match &event.event_type {
        TraceEventType::NoteExported => payload_string(&event.payload, &["title"]),
        TraceEventType::FeedbackRecorded
        | TraceEventType::RankingRecorded
        | TraceEventType::InsightCaptured => payload_string(&event.payload, &["rationale"]),
        _ => None,
    }
}

pub(super) fn event_text(event: &TraceEvent) -> String {
    let mut parts = Vec::new();
    collect_strings(&event.payload, &mut parts);
    parts.join("\n").to_lowercase()
}

pub(super) fn collect_strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(value) => out.push(value.clone()),
        Value::Array(items) => {
            for item in items {
                collect_strings(item, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect_strings(value, out);
            }
        }
        _ => {}
    }
}

pub(super) fn extract_event_tile_id(payload: &Value) -> Option<String> {
    first_payload_string(
        payload,
        &[
            &["tile_id"],
            &["response", "tile_id"],
            &["evidence", "tile_id"],
            &["ranked_responses", "0", "response", "tile_id"],
        ],
    )
}

pub(super) fn extract_event_model_id(payload: &Value) -> Option<String> {
    first_payload_string(
        payload,
        &[
            &["model_id"],
            &["response", "model_id"],
            &["evidence", "model_id"],
            &["ranked_responses", "0", "response", "model_id"],
        ],
    )
}

pub(super) fn first_payload_string(payload: &Value, paths: &[&[&str]]) -> Option<String> {
    paths
        .iter()
        .find_map(|path| payload_string(payload, path))
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

pub(super) fn payload_string(payload: &Value, path: &[&str]) -> Option<String> {
    let mut current = payload;
    for part in path {
        if let Ok(index) = part.parse::<usize>() {
            current = current.as_array()?.get(index)?;
        } else {
            current = current.get(*part)?;
        }
    }

    current
        .as_str()
        .map(ToOwned::to_owned)
        .or_else(|| match current {
            Value::Bool(value) => Some(value.to_string()),
            Value::Number(value) => Some(value.to_string()),
            _ => None,
        })
}

pub(super) fn value_contains_key(value: &Value, key: &str) -> bool {
    match value {
        Value::Object(map) => {
            map.contains_key(key) || map.values().any(|value| value_contains_key(value, key))
        }
        Value::Array(items) => items.iter().any(|value| value_contains_key(value, key)),
        _ => false,
    }
}

pub(super) fn text_contains_any(text: &str, needles: &[&str]) -> bool {
    let text = text.to_lowercase();
    needles.iter().any(|needle| text.contains(needle))
}

pub(super) fn excerpt(content: &str) -> String {
    if content.chars().count() <= EXCERPT_MAX_CHARS {
        return content.to_string();
    }

    let mut excerpt = content.chars().take(EXCERPT_MAX_CHARS).collect::<String>();
    excerpt.push_str("...");
    excerpt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::twin::{default_record_confidence, PromotionState, UserRecordKind};
    use tempfile::tempdir;

    /// Corrupt-file quarantine sweep (Task 1.5): a directory listing must never brick
    /// entirely because of one unparsable file. Covers the two sites named in the audit
    /// (`ensure_record_cache`, `list_session_traces`) plus every sibling loop found with
    /// the same `?`-propagate-on-per-file-parse-failure pattern (`list_decision_episodes`,
    /// `list_constitution_items`, `list_action_gaps`, `list_reflection_cards`).
    mod corrupt_file_quarantine {
        use super::*;

        /// Return the `.corrupt-*` sibling files in `dir`, if any.
        fn quarantined_siblings(dir: &Path) -> Vec<std::path::PathBuf> {
            std::fs::read_dir(dir)
                .expect("read dir")
                .filter_map(|entry| entry.ok())
                .map(|entry| entry.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.contains(".corrupt-"))
                        .unwrap_or(false)
                })
                .collect()
        }

        #[test]
        fn load_or_quarantine_skips_unreadable_file_without_quarantining() {
            let temp_dir = tempdir().expect("temp dir should be created");
            // A directory standing in for a file triggers a genuine read error (not a
            // parse error) deterministically, without relying on OS permission quirks.
            let bogus_path = temp_dir.path().join("not-a-file.json");
            std::fs::create_dir(&bogus_path).expect("create directory standing in for a file");

            let result = load_or_quarantine::<UserRecord>(&bogus_path, "record");
            assert!(result.is_none());
            assert!(
                bogus_path.exists(),
                "a read failure may be transient (permissions, AV lock) — the path must be left untouched"
            );
            assert!(quarantined_siblings(temp_dir.path()).is_empty());
        }

        #[test]
        fn list_user_records_quarantines_corrupt_record_and_returns_healthy_ones() {
            let temp_dir = tempdir().expect("temp dir should be created");
            let mut store = TwinStore::new(temp_dir.path().to_path_buf());

            for i in 0..2 {
                let record = UserRecord {
                    id: format!("record-{i}"),
                    kind: UserRecordKind::Preference,
                    content: format!("Healthy record {i}"),
                    evidence_refs: Vec::new(),
                    confidence: default_record_confidence(),
                    origin: RecordOrigin::User,
                    promotion_state: PromotionState::default_for_origin(&RecordOrigin::User),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    valid_from: None,
                    valid_until: None,
                    links: Vec::new(),
                    metadata: HashMap::new(),
                };
                store
                    .write_record_file(&record)
                    .expect("healthy record should write");
            }

            let corrupt_path = store.records_path.join("truncated-record.json");
            std::fs::write(&corrupt_path, b"{ \"id\": \"broken\", \"content\": \"trunc")
                .expect("seed truncated record file");

            let records = store
                .list_user_records()
                .expect("one corrupt file must not brick the listing");

            assert_eq!(records.len(), 2, "both healthy records should still load");
            assert!(
                !corrupt_path.exists(),
                "corrupt file should be moved out of place"
            );
            assert_eq!(quarantined_siblings(&store.records_path).len(), 1);
        }

        #[test]
        fn list_session_traces_quarantines_corrupt_trace_and_returns_healthy_ones() {
            let temp_dir = tempdir().expect("temp dir should be created");
            let mut store = TwinStore::new(temp_dir.path().to_path_buf());

            store
                .append_trace_event(
                    "session-a",
                    TraceEventType::RankingRecorded,
                    json!({"ranking": ["a"]}),
                )
                .expect("trace a should append");
            store
                .append_trace_event(
                    "session-b",
                    TraceEventType::RankingRecorded,
                    json!({"ranking": ["b"]}),
                )
                .expect("trace b should append");

            let corrupt_path = store.traces_path.join("truncated-trace.json");
            std::fs::write(
                &corrupt_path,
                b"{ \"session_id\": \"broken\", \"events\": [",
            )
            .expect("seed truncated trace file");

            let traces = store
                .list_session_traces()
                .expect("one corrupt file must not brick the listing");

            assert_eq!(traces.len(), 2, "both healthy traces should still load");
            assert!(
                !corrupt_path.exists(),
                "corrupt file should be moved out of place"
            );
            assert_eq!(quarantined_siblings(&store.traces_path).len(), 1);
        }

        #[test]
        fn list_decision_episodes_quarantines_corrupt_episode_and_returns_healthy_ones() {
            let temp_dir = tempdir().expect("temp dir should be created");
            let mut store = TwinStore::new(temp_dir.path().to_path_buf());

            for i in 0..2 {
                store
                    .record_decision_episode(DecisionEpisodeCreate {
                        id: format!("decision-{i}"),
                        session_id: "session-1".to_string(),
                        tile_id: "tile-1".to_string(),
                        decision: format!("Decision {i}"),
                        options: vec!["A".to_string(), "B".to_string()],
                        stakes: None,
                        initial_leaning: None,
                        review_date: None,
                        primitive_assessment: PrimitiveDecisionAssessment::default(),
                        context_version: None,
                    })
                    .expect("healthy decision episode should persist");
            }

            let corrupt_path = store.decisions_path.join("truncated-decision.json");
            std::fs::write(
                &corrupt_path,
                b"{ \"id\": \"broken\", \"decision\": \"trunc",
            )
            .expect("seed truncated decision file");

            let episodes = store
                .list_decision_episodes()
                .expect("one corrupt file must not brick the listing");

            assert_eq!(episodes.len(), 2, "both healthy episodes should still load");
            assert!(
                !corrupt_path.exists(),
                "corrupt file should be moved out of place"
            );
            assert_eq!(quarantined_siblings(&store.decisions_path).len(), 1);
        }

        #[test]
        fn list_constitution_items_quarantines_corrupt_item_and_returns_healthy_ones() {
            let temp_dir = tempdir().expect("temp dir should be created");
            let store = TwinStore::new(temp_dir.path().to_path_buf());

            for i in 0..2 {
                store
                    .create_constitution_item(ConstitutionItemCreate {
                        claim: format!("Claim {i}"),
                        dimension: "general".to_string(),
                        scope: Vec::new(),
                        priority: default_record_confidence(),
                        confidence: default_record_confidence(),
                        status: ConstitutionStatus::Candidate,
                        evidence_refs: Vec::new(),
                        tensions: Vec::new(),
                        linked_record_ids: Vec::new(),
                        source: None,
                    })
                    .expect("healthy constitution item should persist");
            }

            let corrupt_path = store.constitution_path.join("truncated-item.json");
            std::fs::write(&corrupt_path, b"{ \"id\": \"broken\", \"claim\": \"trunc")
                .expect("seed truncated constitution item file");

            let items = store
                .list_constitution_items()
                .expect("one corrupt file must not brick the listing");

            assert_eq!(items.len(), 2, "both healthy items should still load");
            assert!(
                !corrupt_path.exists(),
                "corrupt file should be moved out of place"
            );
            assert_eq!(quarantined_siblings(&store.constitution_path).len(), 1);
        }

        #[test]
        fn list_action_gaps_quarantines_corrupt_gap_and_returns_healthy_ones() {
            let temp_dir = tempdir().expect("temp dir should be created");
            let store = TwinStore::new(temp_dir.path().to_path_buf());

            for i in 0..2 {
                store
                    .create_action_gap(ActionGapCreate {
                        stated_value: format!("Stated {i}"),
                        revealed_behavior: format!("Revealed {i}"),
                        driver_hypothesis: None,
                        somatic_taste_signal: None,
                        decision_risk: "Some risk".to_string(),
                        evidence_refs: Vec::new(),
                        linked_record_ids: Vec::new(),
                        confidence: default_record_confidence(),
                        status: ConstitutionStatus::Candidate,
                    })
                    .expect("healthy action gap should persist");
            }

            let corrupt_path = store.action_gaps_path.join("truncated-gap.json");
            std::fs::write(
                &corrupt_path,
                b"{ \"id\": \"broken\", \"stated_value\": \"trunc",
            )
            .expect("seed truncated action gap file");

            let gaps = store
                .list_action_gaps()
                .expect("one corrupt file must not brick the listing");

            assert_eq!(gaps.len(), 2, "both healthy gaps should still load");
            assert!(
                !corrupt_path.exists(),
                "corrupt file should be moved out of place"
            );
            assert_eq!(quarantined_siblings(&store.action_gaps_path).len(), 1);
        }

        #[test]
        fn list_reflection_cards_quarantines_corrupt_card_and_returns_healthy_ones() {
            let temp_dir = tempdir().expect("temp dir should be created");
            let mut store = TwinStore::new(temp_dir.path().to_path_buf());

            let episode = store
                .record_decision_episode(DecisionEpisodeCreate {
                    id: "decision-1".to_string(),
                    session_id: "session-1".to_string(),
                    tile_id: "tile-1".to_string(),
                    decision: "Should Grafyn build Decision Mirror first?".to_string(),
                    options: vec!["Decision Mirror".to_string(), "Topology".to_string()],
                    stakes: None,
                    initial_leaning: None,
                    review_date: None,
                    primitive_assessment: PrimitiveDecisionAssessment::default(),
                    context_version: None,
                })
                .expect("decision episode should persist");

            for i in 0..2 {
                store
                    .record_reflection_card(ReflectionCardCreate {
                        decision_episode_id: episode.id.clone(),
                        session_id: episode.session_id.clone(),
                        tile_id: episode.tile_id.clone(),
                        model_id: "openai/gpt-4".to_string(),
                        content: format!("## Reflection {i}\nSome content here."),
                        cited_note_ids: Vec::new(),
                        cited_user_record_ids: Vec::new(),
                        cited_constitution_item_ids: Vec::new(),
                        cited_action_gap_ids: Vec::new(),
                        evidence_packet: None,
                    })
                    .expect("healthy reflection card should persist");
            }

            let corrupt_path = store.reflections_path.join("truncated-card.json");
            std::fs::write(&corrupt_path, b"{ \"id\": \"broken\", \"content\": \"trunc")
                .expect("seed truncated reflection card file");

            let cards = store
                .list_reflection_cards()
                .expect("one corrupt file must not brick the listing");

            assert_eq!(cards.len(), 2, "both healthy cards should still load");
            assert!(
                !corrupt_path.exists(),
                "corrupt file should be moved out of place"
            );
            assert_eq!(quarantined_siblings(&store.reflections_path).len(), 1);
        }

        #[test]
        fn decision_evidence_refs_quarantines_corrupt_trace_and_returns_healthy_refs() {
            let temp_dir = tempdir().expect("temp dir should be created");
            let mut store = TwinStore::new(temp_dir.path().to_path_buf());

            // Two healthy traces, each carrying one event tied to the decision.
            store
                .append_trace_event(
                    "session-a",
                    TraceEventType::DecisionEpisodeCreated,
                    json!({"decision_episode_id": "decision-1"}),
                )
                .expect("trace a should append");
            store
                .append_trace_event(
                    "session-b",
                    TraceEventType::FeedbackRecorded,
                    json!({"decision_episode_id": "decision-1"}),
                )
                .expect("trace b should append");

            let corrupt_path = store.traces_path.join("truncated-trace.json");
            std::fs::write(
                &corrupt_path,
                b"{ \"session_id\": \"broken\", \"events\": [",
            )
            .expect("seed truncated trace file");

            let refs = store
                .decision_evidence_refs("decision-1")
                .expect("one corrupt trace must not brick evidence collection");

            assert_eq!(
                refs.len(),
                2,
                "evidence refs from both healthy traces should still be collected"
            );
            let mut session_ids: Vec<_> = refs.iter().map(|r| r.session_id.as_str()).collect();
            session_ids.sort_unstable();
            assert_eq!(session_ids, vec!["session-a", "session-b"]);
            assert!(
                !corrupt_path.exists(),
                "corrupt file should be moved out of place"
            );
            assert_eq!(quarantined_siblings(&store.traces_path).len(), 1);
        }
    }
}
