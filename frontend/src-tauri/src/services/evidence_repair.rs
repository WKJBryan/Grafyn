//! Reversible repair of unreviewed, source-derived personal claims.
//! Callers must hold the Twin write lock and reload its cache after mutation.
//! Sources, decisions, manual records and reviewed items are never rewritten here.
use crate::models::note::Note;
use crate::services::{atomic_io::write_atomic, source_content};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairChange {
    pub relative_path: String,
    pub reason: String,
    pub source_ids: Vec<String>,
    pub before_hash: String,
    pub after_hash: String,
    pub before: String,
    pub after: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairException {
    pub relative_path: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepairManifest {
    pub version: u32,
    pub id: String,
    pub root: String,
    pub changes: Vec<RepairChange>,
    pub review_only: Vec<RepairException>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RepairResult {
    pub changed: usize,
    pub already_applied: usize,
    pub conflicts: Vec<String>,
    pub manifest_path: Option<String>,
}

// Fingerprints aid inspection; exact byte equality, not a noncryptographic hash,
// is the authoritative conflict check below.
fn fingerprint(bytes: &[u8]) -> String {
    let hash = bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
    });
    format!("fnv1a64:{hash:016x}")
}

pub fn preview(root: &Path, notes: &[Note]) -> Result<RepairManifest> {
    let root = root.canonicalize().context("Twin root must exist")?;
    let notes: HashMap<&str, &Note> = notes.iter().map(|note| (note.id.as_str(), note)).collect();
    let mut changes = Vec::new();
    let mut review_only = Vec::new();
    for directory in ["constitution", "records"] {
        let path = root.join(directory);
        if !path.exists() {
            continue;
        }
        let mut entries = std::fs::read_dir(path)?.collect::<std::io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            if entry.path().extension().and_then(|s| s.to_str()) != Some("json")
                || !entry.file_type()?.is_file()
            {
                continue;
            }
            let relative_path = format!("{}/{}", directory, entry.file_name().to_string_lossy());
            let path = checked_path(&root, &relative_path)?;
            let before = std::fs::read_to_string(path)?;
            let value: Value = match serde_json::from_str(&before) {
                Ok(value) => value,
                Err(error) => {
                    review_only.push(RepairException {
                        relative_path,
                        reason: format!("Unreadable derived record: {error}"),
                    });
                    continue;
                }
            };
            if value.get("evidence_repair").is_some() {
                continue;
            }
            let source = value["source"].as_str().unwrap_or_default();
            let personal_record = directory == "records"
                && value["origin"] == "inferred"
                && matches!(
                    value["kind"].as_str(),
                    Some("preference" | "reasoning_pattern")
                );
            let derived_constitution = directory == "constitution"
                && matches!(
                    source,
                    "note_inference"
                        | "interview_behavior_inference"
                        | "interview_research_inference"
                );
            if !personal_record && !derived_constitution {
                continue;
            }
            let refs = value["evidence_refs"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            let mut source_ids = Vec::new();
            let mut navigation = false;
            let mut unresolved = false;
            for receipt in &refs {
                let Some(id) = receipt["source_id"].as_str() else {
                    continue;
                };
                let Some(note) = notes.get(id) else {
                    continue;
                };
                source_ids.push(id.to_string());
                navigation |= generated_wrapper_receipt(
                    note,
                    receipt["excerpt"].as_str().unwrap_or_default(),
                );
                unresolved |= source_content::target_passages(note).is_empty();
            }
            if !navigation && !unresolved {
                continue;
            }
            let reason = if navigation {
                "generated_import_navigation_in_evidence"
            } else {
                "target_attribution_unresolved"
            }
            .to_string();
            let status_key = if personal_record {
                "promotion_state"
            } else {
                "status"
            };
            // Updated timestamps or an accepted/rejected status may represent user review.
            let unreviewed = value[status_key] == "candidate"
                && value["created_at"].is_string()
                && value["created_at"] == value["updated_at"];
            if !unreviewed {
                review_only.push(RepairException {
                    relative_path,
                    reason: format!("{reason}; preserved because review or edits may exist"),
                });
                continue;
            }
            let mut after = value;
            after[status_key] = json!("rejected");
            after["evidence_repair"] = json!({"version":1,"reason":reason,"previous_status":"candidate","source_ids":source_ids});
            let after = serde_json::to_string_pretty(&after)?;
            changes.push(RepairChange {
                relative_path,
                reason,
                source_ids,
                before_hash: fingerprint(before.as_bytes()),
                after_hash: fingerprint(after.as_bytes()),
                before,
                after,
            });
        }
    }
    let id = fingerprint(&serde_json::to_vec(&changes)?).replace(':', "-");
    Ok(RepairManifest {
        version: 1,
        id,
        root: root.to_string_lossy().to_string(),
        changes,
        review_only,
    })
}

fn generated_wrapper_receipt(note: &Note, excerpt: &str) -> bool {
    let kind = note.properties.get("content_kind").and_then(Value::as_str);
    let via = note.properties.get("created_via").and_then(Value::as_str);
    if !matches!(via, Some("content_import" | "document_import"))
        || !matches!(kind, Some("document_section" | "document_index"))
    {
        return false;
    }
    let quote = excerpt
        .trim()
        .trim_end_matches("...")
        .trim_end_matches('…')
        .trim();
    if quote.is_empty() {
        return false;
    }
    let original = note.content.replace("\r\n", "\n");
    if !original.trim().starts_with(quote) {
        return false;
    }
    if kind == Some("document_index") {
        return true;
    }
    let Some((prefix, _)) = original.split_once("\n## Content\n") else {
        return false;
    };
    (quote.starts_with(prefix.trim()) || prefix.trim().starts_with(quote))
        && !source_content::source_body(note).contains(quote)
}

fn checked_path(root: &Path, relative: &str) -> Result<PathBuf> {
    let path = Path::new(relative);
    let parts = path.components().collect::<Vec<_>>();
    if parts.len() != 2
        || !parts
            .iter()
            .all(|part| matches!(part, Component::Normal(_)))
        || !matches!(
            parts[0].as_os_str().to_str(),
            Some("constitution" | "records")
        )
        || path.extension().and_then(|s| s.to_str()) != Some("json")
    {
        bail!("Invalid repair target: {relative}");
    }
    let full = root.join(path).canonicalize()?;
    if !full.starts_with(root) {
        bail!("Repair target escapes twin root: {relative}");
    }
    Ok(full)
}

fn validate(root: &Path, manifest: &RepairManifest) -> Result<PathBuf> {
    let root = root.canonicalize()?;
    if manifest.version != 1
        || root.to_string_lossy() != manifest.root
        || manifest.id.is_empty()
        || !manifest
            .id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-')
    {
        bail!("Repair manifest does not match this twin root");
    }
    for change in &manifest.changes {
        checked_path(&root, &change.relative_path)?;
        if fingerprint(change.before.as_bytes()) != change.before_hash
            || fingerprint(change.after.as_bytes()) != change.after_hash
        {
            bail!("Corrupt repair snapshot: {}", change.relative_path);
        }
    }
    Ok(root)
}

/// Snapshot exact before/after bytes before touching any derived record.
/// A conflict preflight aborts all writes; partial I/O failures can be resumed.
pub fn apply(root: &Path, manifest: &RepairManifest) -> Result<RepairResult> {
    let root = validate(root, manifest)?;
    let mut result = RepairResult::default();
    for change in &manifest.changes {
        let current = std::fs::read(checked_path(&root, &change.relative_path)?)?;
        if current != change.before.as_bytes() && current != change.after.as_bytes() {
            result.conflicts.push(change.relative_path.clone());
        }
    }
    if !result.conflicts.is_empty() {
        return Ok(result);
    }
    if manifest.changes.is_empty() {
        return Ok(result);
    }
    let history = root.join("repair_history");
    std::fs::create_dir_all(&history)?;
    if !history.canonicalize()?.starts_with(&root) {
        bail!("Repair history escapes twin root");
    }
    let snapshot = history.join(format!("{}.json", manifest.id));
    let bytes = serde_json::to_vec_pretty(manifest)?;
    if snapshot.exists() {
        if std::fs::read(&snapshot)? != bytes {
            bail!("Existing repair snapshot differs");
        }
    } else {
        write_atomic(&snapshot, &bytes)?;
    }
    result.manifest_path = Some(snapshot.to_string_lossy().to_string());
    for change in &manifest.changes {
        let path = checked_path(&root, &change.relative_path)?;
        let current = std::fs::read(&path)?;
        if current == change.after.as_bytes() {
            result.already_applied += 1;
            continue;
        }
        if current != change.before.as_bytes() {
            result.conflicts.push(change.relative_path.clone());
            continue;
        }
        write_atomic(&path, change.after.as_bytes())?;
        result.changed += 1;
    }
    Ok(result)
}

/// Restore exact snapshots only when the repaired bytes are still present.
/// Later edits are reported and never overwritten.
pub fn rollback(root: &Path, manifest: &RepairManifest) -> Result<RepairResult> {
    let root = validate(root, manifest)?;
    let mut result = RepairResult::default();
    let snapshot = root
        .join("repair_history")
        .join(format!("{}.json", manifest.id));
    let persisted = std::fs::read(&snapshot).context("Repair has no persisted snapshot")?;
    if persisted != serde_json::to_vec_pretty(manifest)? {
        return Err(anyhow!("Repair snapshot does not match manifest"));
    }
    for change in &manifest.changes {
        let path = checked_path(&root, &change.relative_path)?;
        let current = std::fs::read(&path)?;
        if current == change.before.as_bytes() {
            result.already_applied += 1;
            continue;
        }
        if current != change.after.as_bytes() {
            result.conflicts.push(change.relative_path.clone());
            continue;
        }
        write_atomic(&path, change.before.as_bytes())?;
        result.changed += 1;
    }
    result.manifest_path = Some(snapshot.to_string_lossy().to_string());
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, Vec<Note>) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("constitution")).unwrap();
        let mut note = Note::default();
        note.id = "chapter".into();
        note.content =
            "# Chapter\n\nNext: [[Other]]\n\n## Content\nI prefer careful testing.".into();
        note.properties
            .insert("created_via".into(), json!("content_import"));
        note.properties
            .insert("content_kind".into(), json!("document_section"));
        let candidate = json!({"id":"auto","claim":"Note-backed preference: # Chapter","source":"note_inference","status":"candidate","created_at":"2026-01-01","updated_at":"2026-01-01","evidence_refs":[{"source_id":"chapter","excerpt":"# Chapter\n\nNext: [[Other]]"}]});
        std::fs::write(
            dir.path().join("constitution/auto.json"),
            serde_json::to_string(&candidate).unwrap(),
        )
        .unwrap();
        let mut manual = candidate.clone();
        manual["source"] = json!("manual");
        std::fs::write(
            dir.path().join("constitution/manual.json"),
            serde_json::to_string(&manual).unwrap(),
        )
        .unwrap();
        let mut reviewed = candidate;
        reviewed["status"] = json!("active");
        std::fs::write(
            dir.path().join("constitution/reviewed.json"),
            serde_json::to_string(&reviewed).unwrap(),
        )
        .unwrap();
        (dir, vec![note])
    }
    #[test]
    fn repair_snapshots_once_preserves_manual_and_rolls_back_exact_bytes() {
        let (dir, notes) = fixture();
        let manifest = preview(dir.path(), &notes).unwrap();
        assert_eq!(manifest.changes.len(), 1);
        assert_eq!(manifest.review_only.len(), 1);
        assert!(!dir.path().join("repair_history").exists());
        let original = std::fs::read(dir.path().join("constitution/auto.json")).unwrap();
        let manual = std::fs::read(dir.path().join("constitution/manual.json")).unwrap();
        assert_eq!(apply(dir.path(), &manifest).unwrap().changed, 1);
        assert_eq!(apply(dir.path(), &manifest).unwrap().already_applied, 1);
        assert!(preview(dir.path(), &notes).unwrap().changes.is_empty());
        assert_eq!(
            std::fs::read(dir.path().join("constitution/manual.json")).unwrap(),
            manual
        );
        assert_eq!(rollback(dir.path(), &manifest).unwrap().changed, 1);
        assert_eq!(
            std::fs::read(dir.path().join("constitution/auto.json")).unwrap(),
            original
        );
    }
    #[test]
    fn apply_and_rollback_refuse_later_edits() {
        let (dir, notes) = fixture();
        let manifest = preview(dir.path(), &notes).unwrap();
        apply(dir.path(), &manifest).unwrap();
        std::fs::write(dir.path().join("constitution/auto.json"), "user edited").unwrap();
        assert_eq!(
            rollback(dir.path(), &manifest).unwrap().conflicts,
            vec!["constitution/auto.json"]
        );
        assert_eq!(
            apply(dir.path(), &manifest).unwrap().conflicts,
            vec!["constitution/auto.json"]
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("constitution/auto.json")).unwrap(),
            "user edited"
        );
    }
}
