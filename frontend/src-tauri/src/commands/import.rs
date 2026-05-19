use crate::commands::sync_chunk_index_for_notes;
use crate::models::import::{
    ConversationMetadata, ImportLinkSuggestion, ImportPreview, ImportResult, ParsedConversation,
    ParsedMessage,
};
use crate::models::note::{NoteCreate, NoteStatus};
use crate::services::import::{self, document, semantic_links};
use crate::AppState;
use chrono::Utc;
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::Value;
use std::io::{Cursor, Read};
use std::path::Path;
use tauri::State;
use zip::ZipArchive;

enum ParsedImport {
    Conversations {
        platform: String,
        conversations: Vec<ParsedConversation>,
    },
    Document(document::DocumentImportBatch),
}

/// Preview content in an import file (auto-detects format).
#[tauri::command]
pub async fn preview_import(
    file_path: String,
    _state: State<'_, AppState>,
) -> Result<ImportPreview, String> {
    let parsed = parse_import_file(&file_path).await?;
    let (platform, conversations) = match parsed {
        ParsedImport::Conversations {
            platform,
            conversations,
        } => (platform, conversations),
        ParsedImport::Document(batch) => (
            "document".to_string(),
            batch
                .items
                .iter()
                .map(document_item_to_preview)
                .collect::<Vec<_>>(),
        ),
    };

    let total = conversations.len();

    Ok(ImportPreview {
        items: conversations.clone(),
        conversations,
        platform,
        total_conversations: total,
        total_items: total,
    })
}

/// Import selected content as evidence notes.
#[tauri::command]
pub async fn apply_import(
    file_path: String,
    conversation_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    match parse_import_file(&file_path).await? {
        ParsedImport::Conversations { conversations, .. } => {
            apply_conversation_import(conversations, conversation_ids, state).await
        }
        ParsedImport::Document(batch) => {
            apply_document_import(batch, conversation_ids, state).await
        }
    }
}

async fn apply_conversation_import(
    all_conversations: Vec<ParsedConversation>,
    conversation_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let to_import: Vec<_> = if conversation_ids.is_empty() {
        all_conversations
    } else {
        all_conversations
            .into_iter()
            .filter(|c| conversation_ids.contains(&c.id))
            .collect()
    };

    let mut note_ids = Vec::new();
    let mut created_notes = Vec::new();
    let mut errors = Vec::new();
    let mut skipped = 0;

    for conv in &to_import {
        // Format conversation as markdown
        let markdown = import::format_as_markdown(conv);

        // Build tags: platform + import + suggested tags
        let mut tags = conv.suggested_tags.clone();
        if !tags.contains(&"import".to_string()) {
            tags.push("import".to_string());
        }
        if conv.platform == "interview" && !tags.contains(&"interview".to_string()) {
            tags.push("interview".to_string());
        }
        tags.truncate(5);

        let note_create = NoteCreate {
            title: conv.title.clone(),
            content: markdown,
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Evidence,
            tags,
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: Some("import".to_string()),
            optimizer_managed: false,
            properties: {
                let mut props = std::collections::HashMap::new();
                props.insert(
                    "source".into(),
                    serde_json::Value::String(conv.platform.clone()),
                );
                props.insert(
                    "source_id".into(),
                    serde_json::Value::String(conv.id.clone()),
                );
                props.insert(
                    "created_via".into(),
                    serde_json::Value::String(if conv.platform == "interview" {
                        "interview_import".into()
                    } else {
                        "import".into()
                    }),
                );
                if conv.platform == "interview" {
                    props.insert(
                        "source_type".into(),
                        serde_json::Value::String("interview".into()),
                    );
                    props.insert(
                        "interview_id".into(),
                        serde_json::Value::String(conv.id.clone()),
                    );
                    props.insert(
                        "speaker_role".into(),
                        serde_json::Value::String("mixed".into()),
                    );
                }
                if let Some(created_at) = conv.metadata.created_at {
                    props.insert(
                        "original_created_at".into(),
                        serde_json::Value::String(created_at.to_rfc3339()),
                    );
                }
                props
            },
        };

        // Create the note
        let created = {
            let mut store = state.knowledge_store.write().await;
            match store.create_note(note_create) {
                Ok(note) => note,
                Err(e) => {
                    errors.push(format!("Failed to create '{}': {}", conv.title, e));
                    skipped += 1;
                    continue;
                }
            }
        };

        // Index in search
        {
            let mut search = state.search_service.write().await;
            if let Err(e) = search.index_note(&created) {
                log::error!("Failed to index imported note '{}': {}", created.id, e);
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

        note_ids.push(created.id.clone());
        created_notes.push(created);
    }

    sync_chunk_index_for_notes(state.inner(), &created_notes).await;

    let imported = note_ids.len();
    let message = format!(
        "Imported {} conversation{} as evidence notes",
        imported,
        if imported == 1 { "" } else { "s" }
    );

    Ok(ImportResult {
        imported,
        skipped,
        note_ids,
        errors,
        semantic_link_suggestions: Vec::new(),
        semantic_link_error: None,
        message,
    })
}

async fn apply_document_import(
    batch: document::DocumentImportBatch,
    selected_ids: Vec<String>,
    state: State<'_, AppState>,
) -> Result<ImportResult, String> {
    let to_import = if selected_ids.is_empty() {
        batch.items
    } else {
        batch
            .items
            .into_iter()
            .filter(|item| selected_ids.contains(&item.id))
            .collect::<Vec<_>>()
    };

    let mut note_ids = Vec::new();
    let mut created_notes = Vec::new();
    let mut errors = Vec::new();
    let mut skipped = 0;
    let mut imported_section_titles = Vec::new();
    let mut section_inputs = Vec::new();

    for item in &to_import {
        let mut properties = item.metadata.clone();
        properties.insert("source_id".into(), Value::String(item.id.clone()));

        let note_create = NoteCreate {
            title: item.title.clone(),
            content: item.content.clone(),
            relative_path: None,
            aliases: Vec::new(),
            status: NoteStatus::Evidence,
            tags: item.suggested_tags.clone(),
            schema_version: crate::models::note::CURRENT_NOTE_SCHEMA_VERSION,
            migration_source: Some("content_import".to_string()),
            optimizer_managed: false,
            properties,
        };

        let created = {
            let mut store = state.knowledge_store.write().await;
            match store.create_note(note_create) {
                Ok(note) => note,
                Err(e) => {
                    errors.push(format!("Failed to create '{}': {}", item.title, e));
                    skipped += 1;
                    continue;
                }
            }
        };

        {
            let mut search = state.search_service.write().await;
            if let Err(e) = search.index_note(&created) {
                log::error!("Failed to index imported note '{}': {}", created.id, e);
            }
            if let Err(e) = search.commit() {
                log::error!("Failed to commit after indexing '{}': {}", created.id, e);
            }
        }

        {
            let mut graph = state.graph_index.write().await;
            graph.update_note(&created);
        }

        if item.content_kind == "document_section" {
            imported_section_titles.push(item.title.clone());
            section_inputs.push((item.title.clone(), item.content.clone()));
        }

        note_ids.push(created.id.clone());
        created_notes.push(created);
    }

    sync_chunk_index_for_notes(state.inner(), &created_notes).await;

    let (semantic_link_suggestions, semantic_link_error) =
        suggest_import_links_if_available(state.inner(), section_inputs).await;

    let imported = note_ids.len();
    let section_count = imported_section_titles.len();
    let message = format!(
        "Imported {} content item{} as evidence notes ({} section note{})",
        imported,
        if imported == 1 { "" } else { "s" },
        section_count,
        if section_count == 1 { "" } else { "s" }
    );

    Ok(ImportResult {
        imported,
        skipped,
        note_ids,
        errors,
        semantic_link_suggestions,
        semantic_link_error,
        message,
    })
}

async fn suggest_import_links_if_available(
    state: &AppState,
    section_inputs: Vec<(String, String)>,
) -> (Vec<ImportLinkSuggestion>, Option<String>) {
    if section_inputs.len() < 2 {
        return (Vec::new(), None);
    }

    let base_url = {
        let settings = state.settings_service.read().await;
        settings.get().ollama_base_url.clone()
    };

    match semantic_links::suggest_semantic_links(
        &base_url,
        semantic_links::DEFAULT_IMPORT_LINK_MODEL,
        &section_inputs,
    )
    .await
    {
        Ok(response) => (
            response
                .links
                .into_iter()
                .map(|link| ImportLinkSuggestion {
                    from_title: link.from_title,
                    to_title: link.to_title,
                    reason: link.reason,
                })
                .collect(),
            None,
        ),
        Err(e) => (Vec::new(), Some(e.to_string())),
    }
}

/// Get list of supported import formats.
#[tauri::command]
pub async fn get_supported_formats() -> Vec<String> {
    vec![
        "chatgpt".to_string(),
        "claude".to_string(),
        "grok".to_string(),
        "gemini".to_string(),
        "interview_transcript".to_string(),
        "interview_docx".to_string(),
        "markdown".to_string(),
        "txt".to_string(),
        "docx".to_string(),
        "pdf".to_string(),
    ]
}

async fn parse_import_file(file_path: &str) -> Result<ParsedImport, String> {
    let content = read_import_content(file_path).await?;
    if let Some(platform) = import::detect_platform(&content) {
        let conversations = import::parse_content(&content).map_err(|e| e.to_string())?;
        return Ok(ParsedImport::Conversations {
            platform: platform.to_string(),
            conversations,
        });
    }

    let path = Path::new(file_path);
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(file_path);
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let batch = if extension.eq_ignore_ascii_case("pdf") {
        let outline_titles = extract_pdf_outline_titles(file_path).await;
        document::parse_pdf_document_text(file_name, &content, &outline_titles)
    } else {
        document::parse_document_text(file_name, extension, &content)
    }
    .map_err(|e| format!("Could not import content: {}", e))?;
    Ok(ParsedImport::Document(batch))
}

async fn read_import_content(file_path: &str) -> Result<String, String> {
    let extension = Path::new(file_path)
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();

    if extension == "docx" {
        let bytes = tokio::fs::read(file_path)
            .await
            .map_err(|e| format!("Failed to read file: {}", e))?;
        return extract_docx_text(&bytes);
    }

    if extension == "pdf" {
        let bytes = tokio::fs::read(file_path)
            .await
            .map_err(|e| format!("Failed to read file: {}", e))?;
        return pdf_extract::extract_text_from_mem(&bytes)
            .map(|text| text.trim().to_string())
            .map_err(|e| format!("Failed to extract text from PDF: {}", e))
            .and_then(|text| {
                if text.is_empty() {
                    Err("PDF did not contain readable text".to_string())
                } else {
                    Ok(text)
                }
            });
    }

    tokio::fs::read_to_string(file_path)
        .await
        .map_err(|e| format!("Failed to read file: {}", e))
}

fn document_item_to_preview(item: &document::DocumentImportItem) -> ParsedConversation {
    ParsedConversation {
        id: item.id.clone(),
        title: item.title.clone(),
        platform: "document".to_string(),
        messages: vec![ParsedMessage {
            index: 0,
            role: if item.content_kind == "document_index" {
                "system".to_string()
            } else {
                "source".to_string()
            },
            content: item.content.clone(),
            timestamp: None,
            model: None,
        }],
        metadata: ConversationMetadata {
            platform: "document".to_string(),
            created_at: Some(Utc::now()),
            updated_at: None,
            message_count: 1,
            model_info: vec![item.content_kind.clone()],
        },
        suggested_tags: item.suggested_tags.clone(),
    }
}

async fn extract_pdf_outline_titles(file_path: &str) -> Vec<String> {
    let Ok(bytes) = tokio::fs::read(file_path).await else {
        return Vec::new();
    };
    let Ok(pdf) = lopdf::Document::load_mem(&bytes) else {
        return Vec::new();
    };
    let Ok(toc) = pdf.get_toc() else {
        return Vec::new();
    };

    toc.toc
        .into_iter()
        .map(|entry| entry.title.trim().to_string())
        .filter(|title| !title.is_empty())
        .collect()
}

fn extract_docx_text(bytes: &[u8]) -> Result<String, String> {
    let cursor = Cursor::new(bytes);
    let mut archive =
        ZipArchive::new(cursor).map_err(|e| format!("Failed to open DOCX archive: {}", e))?;
    let mut document = archive
        .by_name("word/document.xml")
        .map_err(|e| format!("Failed to find DOCX document text: {}", e))?;
    let mut xml = String::new();
    document
        .read_to_string(&mut xml)
        .map_err(|e| format!("Failed to read DOCX document text: {}", e))?;

    let mut reader = Reader::from_str(&xml);
    reader.config_mut().trim_text(false);
    let mut text = String::new();
    let mut in_text_run = false;

    loop {
        match reader.read_event() {
            Ok(Event::Start(event)) => match event.name().as_ref() {
                b"w:t" => in_text_run = true,
                b"w:tab" => text.push('\t'),
                b"w:br" => text.push('\n'),
                _ => {}
            },
            Ok(Event::Empty(event)) => match event.name().as_ref() {
                b"w:tab" => text.push('\t'),
                b"w:br" => text.push('\n'),
                _ => {}
            },
            Ok(Event::Text(event)) if in_text_run => {
                let decoded = event
                    .xml_content()
                    .map_err(|e| format!("Failed to decode DOCX text: {}", e))?;
                text.push_str(&decoded);
            }
            Ok(Event::End(event)) => match event.name().as_ref() {
                b"w:t" => in_text_run = false,
                b"w:p" => {
                    if !text.ends_with('\n') {
                        text.push('\n');
                    }
                }
                _ => {}
            },
            Ok(Event::Eof) => break,
            Err(e) => return Err(format!("Failed to parse DOCX document text: {}", e)),
            _ => {}
        }
    }

    let content = text.trim().to_string();
    if content.is_empty() {
        Err("DOCX did not contain readable text".to_string())
    } else {
        Ok(content)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use zip::write::FileOptions;

    #[tokio::test]
    async fn read_import_content_extracts_docx_transcript_text() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("interview.docx");
        let file = File::create(&path).expect("docx");
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("word/document.xml", FileOptions::default())
            .expect("document.xml");
        zip.write_all(
            br#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>Interviewer: How do you decide what to trust?</w:t></w:r></w:p><w:p><w:r><w:t>Expert: I need a real demo first.</w:t></w:r></w:p></w:body></w:document>"#,
        )
        .expect("xml");
        zip.finish().expect("finish docx");

        let content = read_import_content(path.to_string_lossy().as_ref())
            .await
            .expect("docx content");

        assert!(content.contains("Interviewer: How do you decide what to trust?"));
        assert!(content.contains("Expert: I need a real demo first."));
        assert_eq!(import::detect_platform(&content), Some("interview"));
    }
}
