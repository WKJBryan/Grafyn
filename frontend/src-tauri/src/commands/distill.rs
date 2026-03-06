use crate::models::note::{
    DeduplicationAction, DistillRequest, DistillResponse, ExtractionMode, HubCreatePolicy,
    HubUpdate, NoteCreate, NoteStatus, NoteUpdate,
};
use crate::services::openrouter::ChatMessage;
use crate::services::texttiling::{self, TextTilingConfig};
use crate::services::yake;
use crate::AppState;
use lazy_static::lazy_static;
use regex::Regex;
use serde::Deserialize;
use tauri::State;

lazy_static! {
    /// Inline tag pattern: # followed by letter/digit, then letters/digits/hyphens/underscores
    /// Must not be preceded by ` or # (to skip code and headings)
    /// Uses (?m) multiline + character class instead of lookbehind (unsupported by regex crate)
    static ref INLINE_TAG_RE: Regex =
        Regex::new(r"(?m)(?:^|[^#`])#([a-zA-Z0-9][a-zA-Z0-9_/\-]*)").unwrap();
    /// Fenced code block removal
    static ref FENCED_CODE_RE: Regex = Regex::new(r"(?s)```.*?```").unwrap();
    /// Inline code removal
    static ref INLINE_CODE_RE: Regex = Regex::new(r"`[^`]+`").unwrap();
    /// Whitespace run for tag normalization (collapse multiple spaces → single hyphen)
    static ref WHITESPACE_RUN: Regex = Regex::new(r"\s+").unwrap();
}

// ── Tag utilities ──────────────────────────────────────────────────────────

/// Normalize a single tag: lowercase, strip leading #, collapse whitespace→hyphens, strip edge hyphens
fn normalize_tag(tag: &str) -> String {
    let result = tag.trim_start_matches('#').to_lowercase();
    let result = result.trim();
    let result = WHITESPACE_RUN.replace_all(result, "-");
    result.trim_matches('-').to_string()
}

/// Normalize and deduplicate a list of tags
fn normalize_all_tags(tags: &[String]) -> Vec<String> {
    let mut set: std::collections::HashSet<String> = tags.iter().map(|t| normalize_tag(t)).collect();
    // Remove empty strings
    set.remove("");
    let mut sorted: Vec<String> = set.into_iter().collect();
    sorted.sort();
    sorted
}

/// Parse inline #tags from markdown, ignoring code blocks and headings
fn parse_inline_tags(content: &str) -> Vec<String> {
    // 1. Remove fenced code blocks
    let clean = FENCED_CODE_RE.replace_all(content, "");
    // 2. Remove inline code
    let clean = INLINE_CODE_RE.replace_all(&clean, "");
    // 3. Find tags
    let tags: std::collections::HashSet<String> = INLINE_TAG_RE
        .captures_iter(&clean)
        .filter_map(|cap| cap.get(1).map(|m| normalize_tag(m.as_str())))
        .collect();
    tags.into_iter().collect()
}

/// Suggest a hub title from tags, preferring the tag most relevant to the section title.
///
/// Priority: tag whose hyphen-split words appear most often in the title wins.
/// Ties broken by tag length (longer = more specific). Falls back to first significant tag.
fn suggest_hub(title: &str, tags: &[String]) -> Option<String> {
    let title_lower = title.to_lowercase();

    let significant_tags: Vec<&String> = tags
        .iter()
        .filter(|t| *t != "grafyn" && *t != "draft")
        .collect();

    if significant_tags.is_empty() {
        return None;
    }

    // Score each tag by how many of its words appear in the title
    let best = significant_tags
        .iter()
        .map(|tag| {
            let word_matches = tag
                .split('-')
                .filter(|w| !w.is_empty() && title_lower.contains(*w))
                .count();
            (tag, word_matches, tag.len())
        })
        .max_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)))
        .map(|(tag, _, _)| *tag);

    let chosen = best.unwrap_or(&significant_tags[0]);

    // Convert "my-tag" → "Hub: My Tag"
    let title_case: String = chosen
        .split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ");
    Some(format!("Hub: {}", title_case))
}

// ── Candidate extraction (rules-based) ─────────────────────────────────────

struct AtomicCandidate {
    title: String,
    summary: Vec<String>,
    recommended_tags: Vec<String>,
    suggested_hub: Option<String>,
}

/// META_SECTIONS that should be skipped during extraction
const META_SECTIONS: &[&str] = &[
    "metadata",
    "extracted atomic notes",
    "sources",
    "updates",
];

/// Extract up to 5 summary bullet points from a section body
fn extract_summary_bullets(body: &str) -> Vec<String> {
    body.lines()
        .filter_map(|line| {
            let trimmed = line.trim();
            if trimmed.starts_with("- ") {
                Some(trimmed[2..].to_string())
            } else if trimmed.starts_with("* ") {
                Some(trimmed[2..].to_string())
            } else {
                None
            }
        })
        .take(5)
        .collect()
}

/// Build inherited container tags (excluding meta tags)
fn inherited_tags(container_tags: &[String]) -> Vec<String> {
    container_tags
        .iter()
        .filter(|t| !["chat", "canvas-export", "evidence"].contains(&t.as_str()))
        .cloned()
        .collect()
}

/// Merge section tags with inherited tags, sort, and limit to 5
fn merge_and_limit_tags(section_tags: Vec<String>, inherited: &[String]) -> Vec<String> {
    let mut all_tags: std::collections::HashSet<String> = section_tags.into_iter().collect();
    all_tags.extend(inherited.iter().cloned());
    let mut tags: Vec<String> = all_tags.into_iter().collect();
    tags.sort();
    tags.truncate(5);
    tags
}

/// Extract atomic candidates by splitting on H2 headings (rules-based).
///
/// When an H2 section contains H3 sub-headings (e.g. `## Conversation History`
/// with `### Message N` children), performs a secondary split on H3 to produce
/// individual candidates from each sub-section. This handles imported
/// conversation container notes which pack all content under a single H2.
fn extract_candidates_rules(
    content: &str,
    container_tags: &[String],
) -> Vec<AtomicCandidate> {
    let mut candidates = Vec::new();
    let inherited = inherited_tags(container_tags);

    // Split on \n## (keeping content before first ## is skipped)
    let sections: Vec<&str> = content.split("\n## ").collect();

    for section in sections.iter().skip(1) {
        let lines: Vec<&str> = section.lines().collect();
        if lines.is_empty() {
            continue;
        }

        let title = lines[0].trim().to_string();
        let body = lines[1..].join("\n").trim().to_string();

        // Skip meta sections
        if META_SECTIONS.contains(&title.to_lowercase().as_str()) {
            continue;
        }

        // Skip if too short
        if body.len() < 50 {
            continue;
        }

        // Check for H3 sub-headings — if present, extract each as a candidate
        let h3_sections: Vec<&str> = body.split("\n### ").collect();
        if h3_sections.len() > 1 {
            // H3 sub-headings present — extract each as a candidate
            for h3_section in h3_sections.iter().skip(1) {
                let h3_lines: Vec<&str> = h3_section.lines().collect();
                if h3_lines.is_empty() {
                    continue;
                }
                let h3_title = h3_lines[0].trim().to_string();
                let h3_body = h3_lines[1..].join("\n").trim().to_string();

                if h3_body.len() < 50 {
                    continue;
                }

                let summary = extract_summary_bullets(&h3_body);
                let mut section_tags = parse_inline_tags(&h3_body);
                section_tags.extend(yake::extract_tags(&h3_body, 3));
                let section_tags = section_tags;
                let recommended_tags = merge_and_limit_tags(section_tags, &inherited);
                let suggested_hub = suggest_hub(&h3_title, &recommended_tags);

                let summary = if summary.is_empty() {
                    if h3_body.len() > 200 {
                        vec![format!("{}...", &h3_body[..200])]
                    } else {
                        vec![h3_body.clone()]
                    }
                } else {
                    summary
                };

                candidates.push(AtomicCandidate {
                    title: format!("Atomic: {}", h3_title),
                    summary,
                    recommended_tags,
                    suggested_hub,
                });
            }
            continue; // skip H2-level candidate since we extracted H3 children
        }

        // Normal H2 section — extract as single candidate
        let summary = extract_summary_bullets(&body);
        let mut section_tags = parse_inline_tags(&body);
        section_tags.extend(yake::extract_tags(&body, 3));
        let section_tags = section_tags;
        let recommended_tags = merge_and_limit_tags(section_tags, &inherited);
        let suggested_hub = suggest_hub(&title, &recommended_tags);

        let summary = if summary.is_empty() {
            if body.len() > 200 {
                vec![format!("{}...", &body[..200])]
            } else {
                vec![body.clone()]
            }
        } else {
            summary
        };

        candidates.push(AtomicCandidate {
            title: format!("Atomic: {}", title),
            summary,
            recommended_tags,
            suggested_hub,
        });
    }

    candidates
}

// ── LLM structured extraction ─────────────────────────────────────────────

/// LLM response format for structured extraction
#[derive(Debug, Deserialize)]
struct LlmExtractionResponse {
    candidates: Vec<LlmCandidate>,
}

#[derive(Debug, Deserialize)]
struct LlmCandidate {
    title: String,
    summary: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    suggested_hub: Option<String>,
}

/// LLM V2 response format with confidence scoring and recursive splitting
#[derive(Debug, Deserialize)]
struct LlmExtractionResponseV2 {
    candidates: Vec<LlmCandidateV2>,
}

#[derive(Debug, Deserialize)]
struct LlmCandidateV2 {
    title: String,
    summary: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
    suggested_hub: Option<String>,
    #[serde(default)]
    keyphrases: Vec<String>,
    #[serde(default = "default_confidence")]
    confidence: f64,
    #[serde(default)]
    sub_candidates: Option<Vec<LlmCandidateV2>>,
}

fn default_confidence() -> f64 {
    1.0
}

/// Recursively flatten sub_candidates where confidence < 0.7.
fn flatten_candidates(candidates: Vec<LlmCandidateV2>) -> Vec<LlmCandidateV2> {
    let mut flat = Vec::new();
    for c in candidates {
        if c.confidence < 0.7 {
            if let Some(subs) = c.sub_candidates {
                if !subs.is_empty() {
                    flat.extend(flatten_candidates(subs));
                    continue;
                }
            }
        }
        flat.push(LlmCandidateV2 {
            sub_candidates: None,
            ..c
        });
    }
    flat
}

/// Extract JSON from an LLM response, handling ```json wrapping
fn extract_json_from_response(response: &str) -> String {
    let trimmed = response.trim();
    // Handle ```json ... ``` wrapping
    if let Some(start) = trimmed.find("```json") {
        let json_start = start + 7;
        if let Some(end) = trimmed[json_start..].find("```") {
            return trimmed[json_start..json_start + end].trim().to_string();
        }
    }
    // Handle ``` ... ``` wrapping (without language tag)
    if trimmed.starts_with("```") {
        let json_start = trimmed.find('\n').map(|p| p + 1).unwrap_or(3);
        if let Some(end) = trimmed[json_start..].find("```") {
            return trimmed[json_start..json_start + end].trim().to_string();
        }
    }
    trimmed.to_string()
}

/// Extract atomic candidates using structured LLM extraction (JSON response).
///
/// Uses V2 prompt format requesting confidence scoring, keyphrases, and recursive
/// sub_candidates for low-coherence chunks. Falls back to V1 format parsing if
/// the model doesn't handle V2 well.
async fn extract_candidates_llm(
    content: &str,
    container_tags: &[String],
    state: &AppState,
) -> Result<Vec<AtomicCandidate>, String> {
    let system = "You extract structured knowledge from notes. \
        Always respond with valid JSON only, no additional text or markdown formatting.";

    let prompt = format!(
        "Analyze this note and extract distinct atomic knowledge units. \
        Each unit should be a single focused concept, insight, or claim.\n\n\
        Note content:\n---\n{}\n---\n\n\
        Respond with ONLY a JSON object in this exact format:\n\
        {{\"candidates\": [{{\"title\": \"Clear Descriptive Title\", \
        \"summary\": [\"Key point 1\", \"Key point 2\"], \
        \"tags\": [\"lowercase-hyphenated-tag\"], \
        \"suggested_hub\": \"Hub: Topic\", \
        \"keyphrases\": [\"key phrase 1\", \"key phrase 2\"], \
        \"confidence\": 0.85, \
        \"sub_candidates\": null }}]}}\n\n\
        Rules:\n\
        - Set suggested_hub to null if no hub is appropriate\n\
        - Use lowercase hyphenated tags\n\
        - Extract 3-5 keyphrases per chunk (significant terms from the content)\n\
        - Rate confidence 0-1 for how coherent/focused each chunk is\n\
        - If a chunk covers multiple sub-topics (confidence < 0.7), split it \
        into sub_candidates with the same format\n\
        - Extract 1-10 candidates",
        content
    );

    let messages = vec![ChatMessage {
        role: "user".to_string(),
        content: prompt,
    }];

    let model = {
        let settings = state.settings_service.read().await;
        settings.get().llm_model.clone()
    };

    let openrouter = state.openrouter.read().await;
    let response = openrouter
        .chat(
            &model,
            messages,
            Some(system),
            Some(0.3),
            Some(4096),
        )
        .await
        .map_err(|e| format!("LLM extraction failed: {}", e))?;

    let json_str = extract_json_from_response(&response);
    let inherited = inherited_tags(container_tags);

    // Try V2 format first, fall back to V1
    if let Ok(parsed_v2) = serde_json::from_str::<LlmExtractionResponseV2>(&json_str) {
        let flattened = flatten_candidates(parsed_v2.candidates);
        return Ok(flattened
            .into_iter()
            .map(|c| {
                let mut section_tags: Vec<String> =
                    c.tags.iter().map(|t| normalize_tag(t)).collect();
                // Merge 1-2 word keyphrases as additional tag candidates
                for kp in &c.keyphrases {
                    if kp.split_whitespace().count() <= 2 {
                        section_tags.push(normalize_tag(kp));
                    }
                }
                let recommended_tags = merge_and_limit_tags(section_tags, &inherited);
                let suggested_hub =
                    c.suggested_hub.or_else(|| suggest_hub(&c.title, &recommended_tags));

                AtomicCandidate {
                    title: c.title,
                    summary: if c.summary.is_empty() {
                        vec!["No summary provided".to_string()]
                    } else {
                        c.summary
                    },
                    recommended_tags,
                    suggested_hub,
                }
            })
            .collect());
    }

    // V1 fallback
    let parsed: LlmExtractionResponse = serde_json::from_str(&json_str).map_err(|e| {
        format!(
            "Failed to parse LLM JSON: {} (response: {})",
            e,
            &json_str[..json_str.len().min(200)]
        )
    })?;

    Ok(parsed
        .candidates
        .into_iter()
        .map(|c| {
            let section_tags: Vec<String> = c.tags.iter().map(|t| normalize_tag(t)).collect();
            let recommended_tags = merge_and_limit_tags(section_tags, &inherited);
            let suggested_hub =
                c.suggested_hub.or_else(|| suggest_hub(&c.title, &recommended_tags));

            AtomicCandidate {
                title: c.title,
                summary: if c.summary.is_empty() {
                    vec!["No summary provided".to_string()]
                } else {
                    c.summary
                },
                recommended_tags,
                suggested_hub,
            }
        })
        .collect())
}

// ── TextTile extraction ──────────────────────────────────────────────────

/// Extract atomic candidates using TextTiling segmentation + YAKE keyphrases.
///
/// Splits the note content at detected topic boundaries, then uses YAKE to
/// generate titles and tags for each segment.
fn extract_candidates_texttile(
    content: &str,
    container_tags: &[String],
) -> Vec<AtomicCandidate> {
    let config = TextTilingConfig::default();
    let segments = texttiling::segment(content, &config);
    let inherited = inherited_tags(container_tags);

    segments
        .into_iter()
        .filter(|seg| seg.content.split_whitespace().count() >= 30)
        .map(|seg| {
            let title = format!("Atomic: {}", yake::generate_title(&seg.content));
            let yake_tags = yake::extract_tags(&seg.content, 3);
            let inline_tags = parse_inline_tags(&seg.content);
            let section_tags: Vec<String> = yake_tags.into_iter().chain(inline_tags).collect();
            let recommended_tags = merge_and_limit_tags(section_tags, &inherited);
            let suggested_hub = suggest_hub(&title, &recommended_tags);

            let summary = extract_summary_bullets(&seg.content);
            let summary = if summary.is_empty() {
                let truncated: String = seg.content.chars().take(200).collect();
                vec![if seg.content.len() > 200 {
                    format!("{}...", truncated)
                } else {
                    truncated
                }]
            } else {
                summary
            };

            AtomicCandidate {
                title,
                summary,
                recommended_tags,
                suggested_hub,
            }
        })
        .collect()
}

/// Algorithm extraction: heading heuristic (≥2 H2 → rules splitting, else TextTiling).
fn extract_candidates_algorithm(content: &str, tags: &[String]) -> Vec<AtomicCandidate> {
    let h2_count = content.matches("\n## ").count();
    if h2_count >= 2 {
        extract_candidates_rules(content, tags)
    } else {
        extract_candidates_texttile(content, tags)
    }
}

// ── Deduplication ─────────────────────────────────────────────────────────

/// Check for existing notes with the same or very similar title.
/// Returns (id, title) of the matching note if found.
async fn find_duplicate(title: &str, state: &AppState) -> Option<(String, String)> {
    let search = state.search_service.read().await;
    // Strip "Atomic: " prefix for broader matching
    let query = title.trim_start_matches("Atomic: ");
    if let Ok(results) = search.search(query, 5) {
        let candidate_lower = title.to_lowercase();
        let candidate_core = candidate_lower.trim_start_matches("atomic: ");
        for result in results {
            let existing_lower = result.note.title.to_lowercase();
            let existing_core = existing_lower.trim_start_matches("atomic: ");
            if existing_core == candidate_core {
                return Some((result.note.id.clone(), result.note.title.clone()));
            }
        }
    }
    None
}

// ── Hub policy ────────────────────────────────────────────────────────────

/// Determine which candidates should get hubs based on the policy.
/// Returns a Vec parallel to `candidates` — Some(hub_title) or None.
fn apply_hub_policy(
    candidates: &[AtomicCandidate],
    policy: &HubCreatePolicy,
) -> Vec<Option<String>> {
    match policy {
        HubCreatePolicy::Never => vec![None; candidates.len()],
        HubCreatePolicy::Always => candidates.iter().map(|c| c.suggested_hub.clone()).collect(),
        HubCreatePolicy::Auto => {
            // Count tag frequency across all candidates
            let mut tag_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for candidate in candidates {
                for tag in &candidate.recommended_tags {
                    *tag_counts.entry(tag.clone()).or_default() += 1;
                }
            }

            // Only create hubs for tags appearing 3+ times
            let frequent_tags: std::collections::HashSet<String> = tag_counts
                .iter()
                .filter(|(_, count)| **count >= 3)
                .map(|(tag, _)| tag.clone())
                .collect();

            candidates
                .iter()
                .map(|c| {
                    c.suggested_hub.as_ref().and_then(|hub| {
                        let has_frequent =
                            c.recommended_tags.iter().any(|t| frequent_tags.contains(t));
                        if has_frequent {
                            Some(hub.clone())
                        } else {
                            None
                        }
                    })
                })
                .collect()
        }
    }
}

// ── Hub management ─────────────────────────────────────────────────────────

/// Create or update a hub note, returning a HubUpdate if successful
async fn update_hub(
    hub_title: &str,
    atomic_id: &str,
    state: &AppState,
) -> Option<HubUpdate> {
    let hub_id = hub_title
        .replace(' ', "_")
        .replace(':', "")
        .to_lowercase();

    // Check if hub exists
    let hub_exists = {
        let store = state.knowledge_store.read().await;
        store.get_note(&hub_id).ok()
    };

    if let Some(hub) = hub_exists {
        // Update existing hub — add link if not present
        let link = format!("[[{}]]", atomic_id);
        if hub.content.contains(&link) {
            return Some(HubUpdate {
                hub_id,
                hub_title: hub_title.to_string(),
                action: "unchanged".to_string(),
                atomic_ids_added: vec![atomic_id.to_string()],
            });
        }

        let new_content = if hub.content.contains("## Atomic Notes") {
            hub.content.replace(
                "## Atomic Notes",
                &format!("## Atomic Notes\n- {}", link),
            )
        } else {
            format!("{}\n\n## Atomic Notes\n- {}", hub.content, link)
        };

        let update = NoteUpdate {
            content: Some(new_content),
            ..Default::default()
        };

        let updated = {
            let mut store = state.knowledge_store.write().await;
            store.update_note(&hub_id, update).ok()
        };

        if let Some(updated_note) = updated {
            // Update search + graph
            {
                let mut search = state.search_service.write().await;
                let _ = search.index_note(&updated_note);
                let _ = search.commit();
            }
            {
                let mut graph = state.graph_index.write().await;
                graph.update_note(&updated_note);
            }
        }

        Some(HubUpdate {
            hub_id,
            hub_title: hub_title.to_string(),
            action: "updated".to_string(),
            atomic_ids_added: vec![atomic_id.to_string()],
        })
    } else {
        // Create new hub
        let content = format!(
            "# {}\n\n\
            ## Stance / Current Summary\n\
            <!-- Brief overview of this topic -->\n\n\
            ## Atomic Notes\n\
            - [[{}]]\n\n\
            ## Open Questions\n\
            <!-- Unanswered questions -->\n\n\
            ## Related Hubs\n\
            <!-- Links to adjacent topic hubs -->\n",
            hub_title, atomic_id
        );

        let hub_create = NoteCreate {
            title: hub_title.to_string(),
            content,
            status: NoteStatus::Draft,
            tags: vec!["hub".to_string(), "grafyn".to_string()],
            properties: Default::default(),
        };

        let created = {
            let mut store = state.knowledge_store.write().await;
            store.create_note(hub_create).ok()
        };

        if let Some(created_note) = created {
            // Update search + graph
            {
                let mut search = state.search_service.write().await;
                let _ = search.index_note(&created_note);
                let _ = search.commit();
            }
            {
                let mut graph = state.graph_index.write().await;
                graph.update_note(&created_note);
            }

            Some(HubUpdate {
                hub_id: created_note.id,
                hub_title: hub_title.to_string(),
                action: "created".to_string(),
                atomic_ids_added: vec![atomic_id.to_string()],
            })
        } else {
            log::error!("Failed to create hub: {}", hub_title);
            None
        }
    }
}

// ── Main distill command ───────────────────────────────────────────────────

/// Distill a container note into atomic draft notes with configurable
/// extraction mode, deduplication, and hub creation policy.
#[tauri::command]
pub async fn distill_note(
    id: String,
    request: DistillRequest,
    state: State<'_, AppState>,
) -> Result<DistillResponse, String> {
    // 1. Get the container note
    let note = {
        let store = state.knowledge_store.read().await;
        store.get_note(&id).map_err(|e| e.to_string())?
    };

    // 2. Determine extraction strategy
    let use_llm = match request.extraction_mode {
        ExtractionMode::Llm => {
            let openrouter_configured = {
                let or = state.openrouter.read().await;
                or.is_configured()
            };
            if !openrouter_configured {
                return Err(
                    "LLM extraction requested but OpenRouter API key not configured".into(),
                );
            }
            true
        }
        ExtractionMode::Algorithm => false,
    };

    let mut extraction_method_used = if use_llm {
        "llm".to_string()
    } else {
        "algorithm".to_string()
    };

    // 3. Extract candidates
    let mut llm_fallback_msg: Option<String> = None;
    let candidates = if use_llm {
        // Structured LLM extraction with V2 JSON response, fallback to Algorithm
        match extract_candidates_llm(&note.content, &note.tags, &state).await {
            Ok(c) if !c.is_empty() => c,
            Ok(_) => {
                log::info!("LLM returned no candidates, falling back to algorithm");
                llm_fallback_msg =
                    Some("LLM returned no candidates, fell back to algorithm".to_string());
                extraction_method_used = "algorithm".to_string();
                extract_candidates_algorithm(&note.content, &note.tags)
            }
            Err(e) => {
                log::warn!("LLM extraction failed, falling back to algorithm: {}", e);
                llm_fallback_msg = Some(format!(
                    "LLM extraction failed ({}), fell back to algorithm",
                    e
                ));
                extraction_method_used = "algorithm".to_string();
                extract_candidates_algorithm(&note.content, &note.tags)
            }
        }
    } else {
        extract_candidates_algorithm(&note.content, &note.tags)
    };

    if candidates.is_empty() {
        return Ok(DistillResponse {
            created_note_ids: vec![],
            hub_updates: vec![],
            container_updated: false,
            message: "No atomic note candidates found in this note".to_string(),
            extraction_method_used: extraction_method_used.to_string(),
            status: "completed".to_string(),
            skipped_duplicates: 0,
            merged_into: vec![],
        });
    }

    // 4. Apply hub policy — determine which candidates get hubs
    let hub_assignments = apply_hub_policy(&candidates, &request.hub_policy);

    // 5. Create atomic notes (with deduplication)
    let mut created_ids: Vec<String> = Vec::new();
    let mut hub_updates: Vec<HubUpdate> = Vec::new();
    let mut skipped_duplicates: usize = 0;
    let mut merged_into: Vec<String> = Vec::new();

    for (i, candidate) in candidates.iter().enumerate() {
        // Check for duplicates
        if let Some((existing_id, existing_title)) =
            find_duplicate(&candidate.title, &state).await
        {
            match request.dedup_action {
                DeduplicationAction::Skip => {
                    log::info!(
                        "Skipping duplicate: '{}' matches '{}'",
                        candidate.title,
                        existing_title
                    );
                    skipped_duplicates += 1;
                    continue;
                }
                DeduplicationAction::Merge => {
                    // Append summary to existing note
                    let summary_list: String = candidate
                        .summary
                        .iter()
                        .map(|s| format!("- {}", s))
                        .collect::<Vec<_>>()
                        .join("\n");

                    let merge_section = format!(
                        "\n\n## Merged from [[{}]]\n{}\n",
                        note.title, summary_list
                    );

                    let existing_note = {
                        let store = state.knowledge_store.read().await;
                        store.get_note(&existing_id).ok()
                    };

                    if let Some(existing) = existing_note {
                        let update = NoteUpdate {
                            content: Some(format!("{}{}", existing.content, merge_section)),
                            ..Default::default()
                        };

                        let updated = {
                            let mut store = state.knowledge_store.write().await;
                            store.update_note(&existing_id, update).ok()
                        };

                        if let Some(updated_note) = updated {
                            {
                                let mut search = state.search_service.write().await;
                                let _ = search.index_note(&updated_note);
                                let _ = search.commit();
                            }
                            {
                                let mut graph = state.graph_index.write().await;
                                graph.update_note(&updated_note);
                            }
                        }
                    }

                    merged_into.push(existing_id);
                    continue;
                }
                DeduplicationAction::Create => {
                    // Fall through to create new note
                }
            }
        }

        let mut tags = candidate.recommended_tags.clone();
        tags.push("draft".to_string());
        let tags = normalize_all_tags(&tags);

        let summary_list: String = candidate
            .summary
            .iter()
            .map(|s| format!("- {}", s))
            .collect::<Vec<_>>()
            .join("\n");

        let content = format!(
            "# {}\n\n\
            ## TL;DR\n\
            {}\n\n\
            ## Details\n\
            <!-- Expand on the key points here -->\n\n\
            ## Sources\n\
            - [[{}]]\n\n\
            ## Updates\n\
            <!-- Future updates appended here with date headers -->\n",
            candidate.title, summary_list, note.title
        );

        let note_create = NoteCreate {
            title: candidate.title.clone(),
            content,
            status: NoteStatus::Draft,
            tags,
            properties: Default::default(),
        };

        // Create the note
        let created = {
            let mut store = state.knowledge_store.write().await;
            store.create_note(note_create).map_err(|e| e.to_string())?
        };

        // Index in search
        {
            let mut search = state.search_service.write().await;
            if let Err(e) = search.index_note(&created) {
                log::error!("Failed to index atomic note '{}': {}", created.id, e);
            }
            if let Err(e) = search.commit() {
                log::error!("Failed to commit after indexing '{}': {}", created.id, e);
            }
        }

        // Update graph
        {
            let mut graph = state.graph_index.write().await;
            graph.update_note(&created);
        }

        created_ids.push(created.id.clone());

        // Create/update hub if policy allows
        if let Some(ref hub_title) = hub_assignments[i] {
            if let Some(hu) = update_hub(hub_title, &created.id, &state).await {
                hub_updates.push(hu);
            }
        }
    }

    // 6. Update container note with extracted links
    let container_updated = if !created_ids.is_empty() {
        let timestamp = chrono::Utc::now().format("%Y-%m-%d %H:%M").to_string();
        let links: String = created_ids
            .iter()
            .map(|aid| format!("- [[{}]]", aid))
            .collect::<Vec<_>>()
            .join("\n");

        let section_content = format!("\n*Last distilled: {}*\n\n{}\n", timestamp, links);
        let section_header = "## Extracted Atomic Notes";

        let new_content = if let Some(start) = note.content.find(section_header) {
            let after_header = start + section_header.len();
            let end = note.content[after_header..]
                .find("\n## ")
                .map(|pos| after_header + pos)
                .unwrap_or(note.content.len());
            format!(
                "{}\n{}{}",
                &note.content[..after_header],
                section_content,
                &note.content[end..]
            )
        } else {
            format!("{}\n\n{}\n{}", note.content, section_header, section_content)
        };

        let update = NoteUpdate {
            content: Some(new_content),
            ..Default::default()
        };

        let updated = {
            let mut store = state.knowledge_store.write().await;
            store.update_note(&id, update).ok()
        };

        if let Some(updated_note) = updated {
            {
                let mut search = state.search_service.write().await;
                let _ = search.index_note(&updated_note);
                let _ = search.commit();
            }
            {
                let mut graph = state.graph_index.write().await;
                graph.update_note(&updated_note);
            }
            true
        } else {
            false
        }
    } else {
        false
    };

    // 7. Build response message
    let count = created_ids.len();
    let mut parts = vec![format!(
        "Created {} draft atomic notes using {}",
        count, extraction_method_used
    )];
    if skipped_duplicates > 0 {
        parts.push(format!("skipped {} duplicates", skipped_duplicates));
    }
    if !merged_into.is_empty() {
        parts.push(format!(
            "merged into {} existing notes",
            merged_into.len()
        ));
    }
    if let Some(fallback) = llm_fallback_msg {
        parts.push(fallback);
    }
    let message = parts.join(", ");

    Ok(DistillResponse {
        created_note_ids: created_ids,
        hub_updates,
        container_updated,
        message,
        extraction_method_used: extraction_method_used.to_string(),
        status: "completed".to_string(),
        skipped_duplicates,
        merged_into,
    })
}

/// Normalize tags for a note (merges inline #tags into YAML frontmatter)
#[tauri::command]
pub async fn normalize_tags(
    id: String,
    state: State<'_, AppState>,
) -> Result<crate::models::note::Note, String> {
    // Get the note
    let note = {
        let store = state.knowledge_store.read().await;
        store.get_note(&id).map_err(|e| e.to_string())?
    };

    // Parse inline tags from content
    let inline_tags = parse_inline_tags(&note.content);

    // Merge with existing tags
    let mut all_tags: std::collections::HashSet<String> =
        note.tags.iter().map(|t| normalize_tag(t)).collect();
    all_tags.extend(inline_tags);
    all_tags.remove("");

    let merged: Vec<String> = {
        let mut v: Vec<String> = all_tags.into_iter().collect();
        v.sort();
        v
    };

    // Only update if tags changed
    let existing_normalized: Vec<String> = normalize_all_tags(&note.tags);
    if merged == existing_normalized {
        return Ok(note);
    }

    let update = NoteUpdate {
        tags: Some(merged),
        ..Default::default()
    };

    let updated = {
        let mut store = state.knowledge_store.write().await;
        store.update_note(&id, update).map_err(|e| e.to_string())?
    };

    // Update search + graph
    {
        let mut search = state.search_service.write().await;
        let _ = search.index_note(&updated);
        let _ = search.commit();
    }
    {
        let mut graph = state.graph_index.write().await;
        graph.update_note(&updated);
    }

    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::note::HubCreatePolicy;

    #[test]
    fn test_extract_json_from_response_plain() {
        let response = r#"{"candidates": [{"title": "Test", "summary": ["a"], "tags": [], "suggested_hub": null}]}"#;
        let json = extract_json_from_response(response);
        let parsed: LlmExtractionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.candidates.len(), 1);
        assert_eq!(parsed.candidates[0].title, "Test");
    }

    #[test]
    fn test_extract_json_from_response_code_block() {
        let response = "Here is the result:\n```json\n{\"candidates\": [{\"title\": \"Test\", \"summary\": [\"a\"], \"tags\": [], \"suggested_hub\": null}]}\n```";
        let json = extract_json_from_response(response);
        let parsed: LlmExtractionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.candidates.len(), 1);
    }

    #[test]
    fn test_extract_json_from_response_bare_code_block() {
        let response = "```\n{\"candidates\": []}\n```";
        let json = extract_json_from_response(response);
        let parsed: LlmExtractionResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.candidates.len(), 0);
    }

    #[test]
    fn test_hub_policy_never() {
        let candidates = vec![
            AtomicCandidate {
                title: "Test".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
        ];
        let result = apply_hub_policy(&candidates, &HubCreatePolicy::Never);
        assert_eq!(result, vec![None]);
    }

    #[test]
    fn test_hub_policy_always() {
        let candidates = vec![
            AtomicCandidate {
                title: "Test".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
        ];
        let result = apply_hub_policy(&candidates, &HubCreatePolicy::Always);
        assert_eq!(result, vec![Some("Hub: Rust".into())]);
    }

    #[test]
    fn test_hub_policy_auto_below_threshold() {
        // Only 2 candidates share the "rust" tag — below the 3+ threshold
        let candidates = vec![
            AtomicCandidate {
                title: "A".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
            AtomicCandidate {
                title: "B".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
        ];
        let result = apply_hub_policy(&candidates, &HubCreatePolicy::Auto);
        assert_eq!(result, vec![None, None]);
    }

    #[test]
    fn test_hub_policy_auto_above_threshold() {
        // 3 candidates share "rust" tag — meets the 3+ threshold
        let candidates = vec![
            AtomicCandidate {
                title: "A".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
            AtomicCandidate {
                title: "B".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
            AtomicCandidate {
                title: "C".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into(), "wasm".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
        ];
        let result = apply_hub_policy(&candidates, &HubCreatePolicy::Auto);
        assert_eq!(
            result,
            vec![
                Some("Hub: Rust".into()),
                Some("Hub: Rust".into()),
                Some("Hub: Rust".into()),
            ]
        );
    }

    #[test]
    fn test_hub_policy_auto_mixed_tags() {
        // "rust" appears 3 times, "python" only 1 time
        let candidates = vec![
            AtomicCandidate {
                title: "A".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
            AtomicCandidate {
                title: "B".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
            AtomicCandidate {
                title: "C".into(),
                summary: vec![],
                recommended_tags: vec!["rust".into()],
                suggested_hub: Some("Hub: Rust".into()),
            },
            AtomicCandidate {
                title: "D".into(),
                summary: vec![],
                recommended_tags: vec!["python".into()],
                suggested_hub: Some("Hub: Python".into()),
            },
        ];
        let result = apply_hub_policy(&candidates, &HubCreatePolicy::Auto);
        // First 3 get hubs (rust is frequent), last one doesn't (python not frequent)
        assert_eq!(
            result,
            vec![
                Some("Hub: Rust".into()),
                Some("Hub: Rust".into()),
                Some("Hub: Rust".into()),
                None,
            ]
        );
    }

    #[test]
    fn test_rules_extraction_h2_sections() {
        let content = "# Container\n\nIntro text\n\n## First Section\n\nThis is a long enough section body with more than fifty characters of content for testing.\n\n- Point one\n- Point two\n\n## Second Section\n\nAnother section that also has enough content to pass the fifty character minimum threshold.\n\n- Detail A\n- Detail B\n";
        let tags = vec!["test".to_string()];
        let candidates = extract_candidates_rules(content, &tags);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].title, "Atomic: First Section");
        assert_eq!(candidates[1].title, "Atomic: Second Section");
    }

    #[test]
    fn test_rules_extraction_skips_meta_sections() {
        let content = "# Container\n\n## Metadata\n\nThis should be skipped even though it has enough characters to pass the threshold.\n\n## Real Section\n\nThis is a real section with enough content to pass the fifty character minimum threshold.\n";
        let tags = vec![];
        let candidates = extract_candidates_rules(content, &tags);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "Atomic: Real Section");
    }

    #[test]
    fn test_rules_extraction_skips_short_sections() {
        let content = "# Container\n\n## Short\n\nToo short.\n\n## Long Enough\n\nThis section has enough content to pass the fifty character minimum threshold for extraction.\n";
        let tags = vec![];
        let candidates = extract_candidates_rules(content, &tags);
        assert_eq!(candidates.len(), 1);
        assert_eq!(candidates[0].title, "Atomic: Long Enough");
    }
}
