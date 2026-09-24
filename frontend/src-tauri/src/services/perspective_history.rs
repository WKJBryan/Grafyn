//! Explicit, append-only perspective states. Note edits are deliberately not inferred
//! as changes in belief: each state carries the user's statement and evidence IDs.
use crate::services::atomic_io::write_atomic;
use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerspectiveState {
    pub id: String,
    pub perspective_id: String,
    pub title: String,
    pub statement: String,
    pub change_kind: String,
    pub reason: String,
    pub source_note_ids: Vec<String>,
    pub previous_state_id: Option<String>,
    /// When the person held this position (may be entered retrospectively).
    pub effective_at: DateTime<Utc>,
    /// When Grafyn recorded this state; never supplied by the client.
    pub recorded_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct NewPerspectiveState {
    pub perspective_id: Option<String>,
    pub title: String,
    pub statement: String,
    pub change_kind: String,
    pub reason: String,
    pub source_note_ids: Vec<String>,
    pub previous_state_id: Option<String>,
    pub effective_at: DateTime<Utc>,
}

pub struct PerspectiveHistory {
    path: PathBuf,
}

impl PerspectiveHistory {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn list(&self) -> Result<Vec<PerspectiveState>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let mut states = Vec::new();
        for entry in std::fs::read_dir(&self.path)? {
            let entry = entry?;
            if entry.path().extension().and_then(|ext| ext.to_str()) != Some("json") {
                continue;
            }
            let bytes = std::fs::read(entry.path())
                .with_context(|| format!("Could not read perspective state {:?}", entry.path()))?;
            states.push(serde_json::from_slice(&bytes)
                .with_context(|| format!("Invalid perspective state {:?}", entry.path()))?);
        }
        states.sort_by(|a: &PerspectiveState, b| {
            a.effective_at.cmp(&b.effective_at)
                .then(a.recorded_at.cmp(&b.recorded_at))
                .then(a.id.cmp(&b.id))
        });
        Ok(states)
    }

    pub fn append(&self, input: NewPerspectiveState) -> Result<PerspectiveState> {
        let states = self.list()?;
        let title = input.title.trim();
        let statement = input.statement.trim();
        let reason = input.reason.trim();
        if title.is_empty() || statement.is_empty() || input.source_note_ids.is_empty() {
            bail!("Title, statement and at least one source note are required");
        }
        if !["initial", "revised", "qualified", "reversed"].contains(&input.change_kind.as_str()) {
            bail!("Invalid perspective change kind");
        }
        let (perspective_id, previous_state_id) = match (input.perspective_id, input.previous_state_id) {
            (None, None) if input.change_kind == "initial" => (Uuid::new_v4().to_string(), None),
            (Some(id), Some(previous_id)) if input.change_kind != "initial" => {
                let previous = states.iter().find(|state| state.id == previous_id)
                    .context("Previous perspective state does not exist")?;
                if previous.perspective_id != id || input.effective_at < previous.effective_at {
                    bail!("Previous state must belong to this perspective and precede the new state");
                }
                (id, Some(previous_id))
            }
            _ => bail!("A new perspective needs an initial state; changes need a previous state"),
        };
        let state = PerspectiveState {
            id: Uuid::new_v4().to_string(),
            perspective_id,
            title: title.to_string(),
            statement: statement.to_string(),
            change_kind: input.change_kind,
            reason: reason.to_string(),
            source_note_ids: input.source_note_ids,
            previous_state_id,
            effective_at: input.effective_at,
            recorded_at: Utc::now(),
        };
        std::fs::create_dir_all(&self.path)?;
        let path = self.path.join(format!("{}.json", state.id));
        write_atomic(&path, serde_json::to_vec_pretty(&state)?.as_slice())?;
        Ok(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_a_sourced_lineage_without_rewriting_earlier_states() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = PerspectiveHistory::new(dir.path().join("perspectives"));
        let at = Utc::now();
        let initial = store.append(NewPerspectiveState {
            perspective_id: None, title: "Design".into(), statement: "Optimise everything".into(),
            change_kind: "initial".into(), reason: "Original view".into(),
            source_note_ids: vec!["note-a".into()], previous_state_id: None, effective_at: at,
        })?;
        let revised = store.append(NewPerspectiveState {
            perspective_id: Some(initial.perspective_id.clone()), title: "Design".into(),
            statement: "Explore before optimising".into(), change_kind: "revised".into(),
            reason: "Project critique".into(), source_note_ids: vec!["note-b".into()],
            previous_state_id: Some(initial.id.clone()), effective_at: at + chrono::Duration::days(1),
        })?;
        let loaded = PerspectiveHistory::new(dir.path().join("perspectives")).list()?;
        assert_eq!(loaded.len(), 2);
        assert_eq!(loaded[0].statement, "Optimise everything");
        assert_eq!(loaded[1].previous_state_id.as_deref(), Some(initial.id.as_str()));
        assert_eq!(loaded[1].id, revised.id);
        Ok(())
    }

    #[test]
    fn rejects_an_unsupported_lineage() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let store = PerspectiveHistory::new(dir.path().join("perspectives"));
        assert!(store.append(NewPerspectiveState {
            perspective_id: Some("invented".into()), title: "T".into(), statement: "S".into(),
            change_kind: "revised".into(), reason: "".into(), source_note_ids: vec!["n".into()],
            previous_state_id: Some("missing".into()), effective_at: Utc::now(),
        }).is_err());
        Ok(())
    }
}
