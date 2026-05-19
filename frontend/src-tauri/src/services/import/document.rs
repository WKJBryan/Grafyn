use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentImportBatch {
    pub source_title: String,
    pub items: Vec<DocumentImportItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentImportItem {
    pub id: String,
    pub title: String,
    pub content: String,
    pub content_kind: String,
    pub suggested_tags: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, Value>,
}

#[derive(Debug, Clone)]
struct SectionDraft {
    title: String,
    body: String,
    source_url: Option<String>,
    level: usize,
}

pub fn parse_document_text(
    file_name: &str,
    extension: &str,
    content: &str,
) -> Result<DocumentImportBatch> {
    let source_title = source_title_from_file(file_name);
    let extension = extension.trim_start_matches('.').to_ascii_lowercase();
    let sections = match extension.as_str() {
        "md" | "markdown" => markdown_sections(content),
        "docx" => docx_sections(content),
        "pdf" => pdf_sections(content),
        _ => markdown_sections(content).or_else(|| generic_sections(content)),
    }
    .or_else(|| generic_sections(content))
    .ok_or_else(|| anyhow!("Document did not contain readable text"))?;

    Ok(build_batch(source_title, file_name, extension, sections))
}

pub fn parse_pdf_document_text(
    file_name: &str,
    content: &str,
    outline_titles: &[String],
) -> Result<DocumentImportBatch> {
    let source_title = source_title_from_file(file_name);
    let sections = pdf_sections_with_outlines(content, outline_titles)
        .or_else(|| pdf_sections(content))
        .or_else(|| generic_sections(content))
        .ok_or_else(|| anyhow!("Document did not contain readable text"))?;
    Ok(build_batch(
        source_title,
        file_name,
        "pdf".to_string(),
        sections,
    ))
}

fn source_title_from_file(file_name: &str) -> String {
    Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(file_name)
        .trim()
        .to_string()
}

fn build_batch(
    source_title: String,
    file_name: &str,
    extension: String,
    mut sections: Vec<SectionDraft>,
) -> DocumentImportBatch {
    make_titles_unique(&mut sections);

    let section_titles = sections
        .iter()
        .map(|section| section.title.clone())
        .collect::<Vec<_>>();
    let mut items = Vec::new();
    let parent_id = stable_id("document-parent", file_name, &source_title);
    let mut parent_metadata = base_metadata(file_name, &extension, "document_index");
    parent_metadata.insert(
        "section_count".to_string(),
        Value::from(section_titles.len()),
    );

    items.push(DocumentImportItem {
        id: parent_id,
        title: source_title.clone(),
        content: format_index_note(&source_title, file_name, &section_titles),
        content_kind: "document_index".to_string(),
        suggested_tags: vec![
            "import".to_string(),
            "document".to_string(),
            "evidence".to_string(),
        ],
        metadata: parent_metadata,
    });

    for (idx, section) in sections.iter().enumerate() {
        let mut metadata = base_metadata(file_name, &extension, "document_section");
        metadata.insert("section_index".to_string(), Value::from(idx));
        metadata.insert("section_level".to_string(), Value::from(section.level));
        if let Some(url) = section.source_url.as_ref() {
            metadata.insert("source_url".to_string(), Value::String(url.clone()));
        }

        items.push(DocumentImportItem {
            id: stable_id(
                "document-section",
                file_name,
                &format!("{}:{}", idx, section.title),
            ),
            title: section.title.clone(),
            content: format_section_note(&source_title, &section_titles, idx, section),
            content_kind: "document_section".to_string(),
            suggested_tags: vec![
                "import".to_string(),
                "document".to_string(),
                "evidence".to_string(),
            ],
            metadata,
        });
    }

    DocumentImportBatch {
        source_title,
        items,
    }
}

fn base_metadata(file_name: &str, extension: &str, content_kind: &str) -> HashMap<String, Value> {
    HashMap::from([
        ("source".to_string(), Value::String("document".to_string())),
        (
            "source_type".to_string(),
            Value::String("document".to_string()),
        ),
        (
            "source_file_name".to_string(),
            Value::String(file_name.to_string()),
        ),
        (
            "source_extension".to_string(),
            Value::String(extension.to_string()),
        ),
        (
            "content_kind".to_string(),
            Value::String(content_kind.to_string()),
        ),
        (
            "created_via".to_string(),
            Value::String("content_import".to_string()),
        ),
    ])
}

fn format_index_note(source_title: &str, file_name: &str, section_titles: &[String]) -> String {
    let links = section_titles
        .iter()
        .map(|title| format!("- [[{}]]", title))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "# {}\n\nImported from `{}` as section-level evidence notes.\n\n## Sections\n{}\n",
        source_title, file_name, links
    )
}

fn format_section_note(
    source_title: &str,
    section_titles: &[String],
    idx: usize,
    section: &SectionDraft,
) -> String {
    let mut lines = vec![
        format!("# {}", section.title),
        String::new(),
        format!("Part of: [[{}]]", source_title),
    ];
    if idx > 0 {
        lines.push(format!("Previous: [[{}]]", section_titles[idx - 1]));
    }
    if idx + 1 < section_titles.len() {
        lines.push(format!("Next: [[{}]]", section_titles[idx + 1]));
    }
    if let Some(url) = section.source_url.as_ref() {
        lines.push(format!("Source: {}", url));
    }
    lines.push(String::new());
    lines.push("## Content".to_string());
    lines.push(section.body.trim().to_string());
    lines.push(String::new());
    lines.join("\n")
}

fn markdown_sections(content: &str) -> Option<Vec<SectionDraft>> {
    let lines = normalized_lines(content);
    let mut sections = Vec::new();
    let mut current: Option<SectionDraft> = None;

    for line in lines {
        if let Some((level, title)) = markdown_heading(&line) {
            flush_section(&mut sections, &mut current);
            current = Some(SectionDraft {
                title,
                body: String::new(),
                source_url: None,
                level,
            });
            continue;
        }
        if let Some(section) = current.as_mut() {
            push_body_line(&mut section.body, &line);
        }
    }
    flush_section(&mut sections, &mut current);

    if sections.is_empty() {
        None
    } else {
        Some(sections)
    }
}

fn docx_sections(content: &str) -> Option<Vec<SectionDraft>> {
    let lines = normalized_lines(content);
    let mut sections = Vec::new();
    let mut current: Option<SectionDraft> = None;
    let mut pending_url: Option<String> = None;

    for line in lines {
        if is_url(&line) {
            flush_section(&mut sections, &mut current);
            pending_url = Some(line);
            continue;
        }

        if let Some(url) = pending_url.take() {
            if is_title_like_after_url(&line) {
                current = Some(SectionDraft {
                    title: line,
                    body: String::new(),
                    source_url: Some(url),
                    level: 1,
                });
                continue;
            }
            current = Some(SectionDraft {
                title: line.chars().take(90).collect(),
                body: String::new(),
                source_url: Some(url),
                level: 1,
            });
            continue;
        }

        if let Some(section) = current.as_mut() {
            push_body_line(&mut section.body, &line);
        }
    }
    flush_section(&mut sections, &mut current);

    if sections.is_empty() {
        None
    } else {
        Some(sections)
    }
}

fn pdf_sections(content: &str) -> Option<Vec<SectionDraft>> {
    let lines = trim_sources_tail(normalized_lines(content));
    let heading_indices = pdf_heading_indices(&lines);
    pdf_sections_from_heading_indices(lines, heading_indices)
}

fn pdf_sections_with_outlines(
    content: &str,
    outline_titles: &[String],
) -> Option<Vec<SectionDraft>> {
    if outline_titles.is_empty() {
        return None;
    }
    let lines = trim_sources_tail(normalized_lines(content));
    let outline_set = outline_titles
        .iter()
        .map(|title| title.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<HashSet<_>>();
    let heading_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| {
            if outline_set.contains(line) {
                Some(idx)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    pdf_sections_from_heading_indices(lines, heading_indices)
}

fn pdf_sections_from_heading_indices(
    lines: Vec<String>,
    heading_indices: Vec<usize>,
) -> Option<Vec<SectionDraft>> {
    if heading_indices.is_empty() {
        return None;
    }

    let mut sections = Vec::new();
    for (pos, &idx) in heading_indices.iter().enumerate() {
        let end = heading_indices.get(pos + 1).copied().unwrap_or(lines.len());
        let body = lines[(idx + 1)..end].join("\n");
        sections.push(SectionDraft {
            title: lines[idx].clone(),
            body,
            source_url: None,
            level: 1,
        });
    }
    Some(sections)
}

fn generic_sections(content: &str) -> Option<Vec<SectionDraft>> {
    let body = normalized_lines(content).join("\n");
    if body.trim().is_empty() {
        None
    } else {
        Some(vec![SectionDraft {
            title: "Imported Content".to_string(),
            body,
            source_url: None,
            level: 1,
        }])
    }
}

fn normalized_lines(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .map(|line| line.trim().to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

fn markdown_heading(line: &str) -> Option<(usize, String)> {
    let trimmed = line.trim_start();
    let level = trimmed.chars().take_while(|ch| *ch == '#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let title = trimmed[level..].trim();
    if title.is_empty() {
        None
    } else {
        Some((level, title.to_string()))
    }
}

fn trim_sources_tail(lines: Vec<String>) -> Vec<String> {
    if let Some(idx) = lines
        .iter()
        .position(|line| line.trim_end_matches(':').eq_ignore_ascii_case("sources"))
    {
        lines[..idx].to_vec()
    } else {
        lines
    }
}

fn pdf_heading_indices(lines: &[String]) -> Vec<usize> {
    let mut indices = Vec::new();
    let mut seen = HashSet::new();
    for (idx, line) in lines.iter().enumerate() {
        if seen.contains(line) {
            continue;
        }
        let Some(next) = lines.get(idx + 1) else {
            continue;
        };
        if is_pdf_heading(line, next) {
            seen.insert(line.clone());
            indices.push(idx);
        }
    }
    indices
}

fn is_pdf_heading(line: &str, next_line: &str) -> bool {
    let word_count = line.split_whitespace().count();
    if !(2..=10).contains(&word_count) {
        return false;
    }
    if line.ends_with('.') || line.ends_with(',') || line.ends_with(';') || line.contains(':') {
        return false;
    }
    if line
        .chars()
        .all(|ch| ch.is_ascii_digit() || ch.is_whitespace())
    {
        return false;
    }
    if next_line.split_whitespace().count() < 6 {
        return false;
    }
    let lower = line.to_ascii_lowercase();
    let has_known_heading_word = [
        "style",
        "values",
        "priorities",
        "evolution",
        "perspective",
        "views",
        "education",
        "work",
    ]
    .iter()
    .any(|needle| lower.contains(needle));
    has_known_heading_word || title_case_ratio(line) >= 0.58 || line.contains("(AI)")
}

fn title_case_ratio(line: &str) -> f64 {
    let words = line.split_whitespace().collect::<Vec<_>>();
    if words.is_empty() {
        return 0.0;
    }
    let title_words = words
        .iter()
        .filter(|word| {
            word.chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit() || ch == '(')
        })
        .count();
    title_words as f64 / words.len() as f64
}

fn is_url(line: &str) -> bool {
    line.starts_with("http://") || line.starts_with("https://")
}

fn is_title_like_after_url(line: &str) -> bool {
    let word_count = line.split_whitespace().count();
    word_count <= 25 && (title_case_ratio(line) >= 0.45 || !line.ends_with('.'))
}

fn push_body_line(body: &mut String, line: &str) {
    if !body.is_empty() {
        body.push('\n');
    }
    body.push_str(line);
}

fn flush_section(sections: &mut Vec<SectionDraft>, current: &mut Option<SectionDraft>) {
    if let Some(section) = current.take() {
        if !section.body.trim().is_empty() {
            sections.push(section);
        }
    }
}

fn make_titles_unique(sections: &mut [SectionDraft]) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for section in sections {
        let count = counts.entry(section.title.clone()).or_insert(0);
        *count += 1;
        if *count > 1 {
            section.title = format!("{} ({})", section.title, count);
        }
    }
}

fn stable_id(prefix: &str, file_name: &str, value: &str) -> String {
    let mut hasher = DefaultHasher::new();
    file_name.hash(&mut hasher);
    value.hash(&mut hasher);
    format!("{}-{:016x}", prefix, hasher.finish())
}
