use crate::models::import::ParsedConversation;
use crate::services::import::{self, document};
use quick_xml::{events::Event, Reader};
use std::io::{Cursor, Read};
use std::path::Path;
use zip::ZipArchive;

pub enum ParsedImport {
    Conversations {
        platform: String,
        conversations: Vec<ParsedConversation>,
    },
    Document(document::DocumentImportBatch),
}

pub async fn parse_import_file(
    file_path: &str,
    mapping: Option<&crate::models::import::ImportTableMapping>,
) -> Result<ParsedImport, String> {
    let path = Path::new(file_path);
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if extension.eq_ignore_ascii_case("csv") || extension.eq_ignore_ascii_case("xlsx") {
        let bytes = tokio::fs::read(file_path)
            .await
            .map_err(|e| e.to_string())?;
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(file_path);
        return super::decision_table::parse_with_mapping(
            name,
            &bytes,
            extension.eq_ignore_ascii_case("xlsx"),
            mapping,
        )
        .map(ParsedImport::Document)
        .map_err(|e| e.to_string());
    }
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

pub async fn read_import_content(file_path: &str) -> Result<String, String> {
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
                let decoded = std::str::from_utf8(event.as_ref())
                    .map_err(|e| format!("Failed to decode DOCX text: {}", e))?;
                text.push_str(decoded);
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
