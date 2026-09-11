//! Source text boundaries. Generated import furniture is never evidence.
use crate::models::note::Note;

pub fn source_body(note: &Note) -> String {
    note.content[source_body_range(note)].to_string()
}

/// Byte range in the unmodified note, so chunk locators remain source-grounded.
pub fn source_body_range(note: &Note) -> std::ops::Range<usize> {
    let provenance = note.properties.get("created_via").and_then(|v| v.as_str());
    let kind = note.properties.get("content_kind").and_then(|v| v.as_str());
    if matches!(provenance, Some("content_import" | "document_import")) {
        if kind == Some("document_index") {
            return 0..0;
        }
        if kind == Some("document_section") {
            for delimiter in ["\n## Content\n", "\n## Content\r\n"] {
                if let Some((prefix, body)) = note.content.split_once(delimiter) {
                    let start =
                        prefix.len() + delimiter.len() + body.len() - body.trim_start().len();
                    return start..start + body.trim().len();
                }
            }
            // An edited or legacy wrapper with no recognizable boundary is unresolved.
            return 0..0;
        }
    }
    0..note.content.len()
}

/// Explicit speaker mapping only; message role and importing user imply no identity.
pub fn target_passages(note: &Note) -> Vec<(String, String)> {
    if note
        .properties
        .get("target_person_id")
        .and_then(|v| v.as_str())
        .is_none_or(|s| s.trim().is_empty())
    {
        return Vec::new();
    }
    let Some(target) = note
        .properties
        .get("target_speaker")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
    else {
        return Vec::new();
    };
    let body = source_body(note);
    if target.eq_ignore_ascii_case("source") {
        return vec![(target.to_string(), body)];
    }
    let mut passages = Vec::new();
    let mut speaker = String::new();
    let mut text = String::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let heading = if trimmed.starts_with("### Message ") || trimmed.starts_with("## Message ") {
            trimmed.split_once(':').map(|(_, label)| {
                let label = label.trim();
                label
                    .rsplit_once(" (")
                    .and_then(|(_, name)| name.strip_suffix(')'))
                    .unwrap_or(label)
                    .to_string()
            })
        } else if let Some((label, _)) = trimmed.split_once(':') {
            if label.trim().eq_ignore_ascii_case(target)
                || matches!(
                    label.trim().to_ascii_lowercase().as_str(),
                    "interviewer"
                        | "interviewee"
                        | "participant"
                        | "expert"
                        | "researcher"
                        | "user"
                        | "assistant"
                )
                // An unrecognized named turn is a boundary, not continuation of
                // the mapped person's answer. Ambiguous prose is conservatively
                // excluded rather than attributed to the previous speaker.
                || (!label.trim().is_empty()
                    && label.len() <= 40
                    && label.split_whitespace().count() <= 4
                    && label.chars().all(|c| {
                        c.is_alphanumeric() || c.is_whitespace() || matches!(c, '-' | '_' | '.')
                    }))
            {
                Some(label.trim().to_string())
            } else {
                None
            }
        } else {
            None
        };
        if let Some(next) = heading {
            if speaker.eq_ignore_ascii_case(target) && !text.trim().is_empty() {
                passages.push((speaker.clone(), text.trim().to_string()));
            }
            text.clear();
            speaker = next;
            if !trimmed.starts_with('#') {
                text.push_str(trimmed.split_once(':').unwrap().1.trim());
                text.push('\n');
            }
        } else if !speaker.is_empty() {
            text.push_str(line);
            text.push('\n');
        }
    }
    if speaker.eq_ignore_ascii_case(target) && !text.trim().is_empty() {
        passages.push((speaker, text.trim().to_string()));
    }
    passages
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn lowercase_and_hyphenated_speakers_cannot_inherit_target_attribution() {
        let mut note = Note::default();
        note.properties
            .insert("target_person_id".into(), json!("person-alex"));
        note.properties
            .insert("target_speaker".into(), json!("Alex"));
        for other in ["bob", "model-x"] {
            note.content =
                format!("Alex: I prefer careful testing.\n{other}: I prefer immediate release.");
            assert_eq!(
                target_passages(&note),
                vec![("Alex".into(), "I prefer careful testing.".into())]
            );
        }
    }
    #[test]
    fn removes_only_proven_import_furniture_and_keeps_authored_links() {
        let mut note = Note::default();
        note.content = "# Title\n\nNext: [[Navigation]]\n\n## Content\nI prefer [[Careful testing]].\nNext: [[Authored followup]]".into();
        assert_eq!(source_body(&note), note.content);
        note.properties
            .insert("created_via".into(), json!("content_import"));
        note.properties
            .insert("content_kind".into(), json!("document_section"));
        let body = source_body(&note);
        assert!(!body.contains("Navigation"));
        assert!(body.contains("[[Authored followup]]"));
        note.properties
            .insert("content_kind".into(), json!("document_index"));
        assert!(source_body(&note).is_empty());
    }
    #[test]
    fn target_can_be_either_speaker_and_unknown_mapping_yields_no_passages() {
        let mut note = Note::default();
        note.content="### Message 1: User (Interviewer)\n\nI prefer concrete examples.\n\n### Message 2: Interviewee (Expert)\n\nI need a working demo.".into();
        assert!(target_passages(&note).is_empty());
        note.properties
            .insert("target_person_id".into(), json!("person-one"));
        note.properties
            .insert("target_speaker".into(), json!("Expert"));
        assert_eq!(
            target_passages(&note),
            vec![("Expert".into(), "I need a working demo.".into())]
        );
        note.properties
            .insert("target_speaker".into(), json!("Interviewer"));
        assert_eq!(
            target_passages(&note),
            vec![("Interviewer".into(), "I prefer concrete examples.".into())]
        );
    }

    #[test]
    fn an_unrecognized_named_turn_cannot_inherit_target_attribution() {
        let mut note = Note::default();
        note.content = "Alex: I prefer careful testing.\nBlair: I prefer immediate release.".into();
        note.properties
            .insert("target_person_id".into(), json!("person-alex"));
        note.properties
            .insert("target_speaker".into(), json!("Alex"));
        assert_eq!(
            target_passages(&note),
            vec![("Alex".into(), "I prefer careful testing.".into())]
        );
    }
}
