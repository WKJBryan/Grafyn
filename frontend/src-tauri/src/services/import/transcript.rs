use crate::models::import::{ConversationMetadata, ParsedConversation, ParsedMessage};
use anyhow::Result;
use chrono::Utc;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
struct TranscriptTurn {
    role: String,
    speaker: String,
    content: String,
}

pub fn can_parse(content: &str) -> bool {
    parse_turns(content).is_some_and(|turns| {
        let has_interviewer = turns.iter().any(|turn| turn.role == "user");
        let has_interviewee = turns.iter().any(|turn| turn.role == "interviewee");
        has_interviewer && has_interviewee
    })
}

pub fn parse(content: &str) -> Result<Vec<ParsedConversation>> {
    let turns = parse_turns(content).ok_or_else(|| {
        anyhow::anyhow!(
            "Transcript import requires clear speaker labels such as 'Interviewer:' and 'Participant:'"
        )
    })?;
    let messages = turns
        .iter()
        .enumerate()
        .map(|(index, turn)| ParsedMessage {
            index,
            role: turn.role.clone(),
            content: turn.content.clone(),
            timestamp: None,
            model: Some(turn.speaker.clone()),
        })
        .collect::<Vec<_>>();
    let title = infer_title(content).unwrap_or_else(|| "Interview Transcript".to_string());
    let id = stable_transcript_id(content);
    let message_count = messages.len();

    Ok(vec![ParsedConversation {
        id,
        title,
        platform: "interview".to_string(),
        messages,
        metadata: ConversationMetadata {
            platform: "interview".to_string(),
            created_at: None,
            updated_at: Some(Utc::now()),
            message_count,
            model_info: vec!["labeled transcript".to_string()],
        },
        suggested_tags: vec!["interview".to_string(), "evidence".to_string()],
    }])
}

fn parse_turns(content: &str) -> Option<Vec<TranscriptTurn>> {
    let mut turns: Vec<TranscriptTurn> = Vec::new();
    let mut current_speaker: Option<String> = None;
    let mut current_role: Option<String> = None;
    let mut current_content = String::new();

    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() {
            if !current_content.is_empty() {
                current_content.push('\n');
            }
            continue;
        }

        if let Some((speaker, body)) = split_speaker_line(line) {
            flush_turn(
                &mut turns,
                &mut current_speaker,
                &mut current_role,
                &mut current_content,
            );
            current_speaker = Some(speaker.to_string());
            current_role = Some(role_for_speaker(speaker));
            current_content.push_str(body.trim());
            continue;
        }

        if current_speaker.is_none() {
            continue;
        }
        if !current_content.is_empty() && !current_content.ends_with('\n') {
            current_content.push(' ');
        }
        current_content.push_str(line);
    }

    flush_turn(
        &mut turns,
        &mut current_speaker,
        &mut current_role,
        &mut current_content,
    );

    if turns.len() >= 2 {
        Some(turns)
    } else {
        None
    }
}

fn flush_turn(
    turns: &mut Vec<TranscriptTurn>,
    current_speaker: &mut Option<String>,
    current_role: &mut Option<String>,
    current_content: &mut String,
) {
    let Some(speaker) = current_speaker.take() else {
        return;
    };
    let role = current_role
        .take()
        .unwrap_or_else(|| "interviewee".to_string());
    let content = current_content.trim().to_string();
    current_content.clear();
    if content.is_empty() {
        return;
    }
    turns.push(TranscriptTurn {
        role,
        speaker,
        content,
    });
}

fn split_speaker_line(line: &str) -> Option<(&str, &str)> {
    let (speaker, body) = line.split_once(':')?;
    let speaker = speaker.trim();
    if speaker.len() > 40 || speaker.split_whitespace().count() > 4 {
        return None;
    }
    let normalized = speaker.to_lowercase();
    let known = matches!(
        normalized.as_str(),
        "interviewer"
            | "interview"
            | "researcher"
            | "moderator"
            | "facilitator"
            | "me"
            | "user"
            | "participant"
            | "interviewee"
            | "respondent"
            | "customer"
            | "student"
            | "teacher"
            | "expert"
    );
    if known {
        Some((speaker, body))
    } else {
        None
    }
}

fn role_for_speaker(speaker: &str) -> String {
    match speaker.to_lowercase().as_str() {
        "interviewer" | "interview" | "researcher" | "moderator" | "facilitator" | "me"
        | "user" => "user".to_string(),
        _ => "interviewee".to_string(),
    }
}

fn infer_title(content: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let title = line.trim().trim_start_matches('#').trim();
        if title.is_empty() || title.contains(':') {
            None
        } else {
            Some(title.chars().take(80).collect())
        }
    })
}

fn stable_transcript_id(content: &str) -> String {
    let mut hasher = DefaultHasher::new();
    content.hash(&mut hasher);
    format!("interview-{:016x}", hasher.finish())
}
