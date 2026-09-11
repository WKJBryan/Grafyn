//! Deterministic decision-table import. Empty answers stay empty; model outputs are separate.
use super::document::{DocumentImportBatch, DocumentImportItem};
use anyhow::{anyhow, Context, Result};
use quick_xml::{events::Event, Reader};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::hash::{Hash, Hasher};
use std::io::{Cursor, Read};

type Row = BTreeMap<String, String>;

pub fn parse(file_name: &str, bytes: &[u8], xlsx: bool) -> Result<DocumentImportBatch> {
    parse_with_mapping(file_name, bytes, xlsx, None)
}

pub fn parse_with_mapping(
    file_name: &str,
    bytes: &[u8],
    xlsx: bool,
    mapping: Option<&crate::models::import::ImportTableMapping>,
) -> Result<DocumentImportBatch> {
    let sheets = if xlsx {
        read_xlsx(bytes)?
    } else {
        let text = std::str::from_utf8(bytes).context("CSV must be UTF-8")?;
        vec![("CSV".to_string(), csv_rows(text)?)]
    };
    build_batch(file_name, sheets, mapping)
}

fn csv_rows(text: &str) -> Result<Vec<(usize, Row)>> {
    let mut records = Vec::new();
    let mut record = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.trim_start_matches('\u{feff}').chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => record.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            '\r' if !quoted => {}
            _ => field.push(ch),
        }
    }
    if quoted {
        return Err(anyhow!("Unclosed quoted CSV cell"));
    }
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    Ok(records
        .into_iter()
        .enumerate()
        .map(|(idx, cells)| {
            (
                idx + 1,
                cells
                    .into_iter()
                    .enumerate()
                    .map(|(col, value)| (column_name(col), value))
                    .collect(),
            )
        })
        .collect())
}

fn column_name(mut index: usize) -> String {
    let mut result = String::new();
    loop {
        result.insert(0, (b'A' + (index % 26) as u8) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    result
}

fn archive_text(archive: &mut zip::ZipArchive<Cursor<&[u8]>>, path: &str) -> Result<String> {
    let mut text = String::new();
    archive.by_name(path)?.read_to_string(&mut text)?;
    Ok(text)
}

fn decode_reference(bytes: &[u8]) -> Result<String> {
    Ok(quick_xml::escape::unescape(&format!("&{};", std::str::from_utf8(bytes)?))?.into_owned())
}

fn attr(event: &quick_xml::events::BytesStart<'_>, key: &[u8]) -> Option<String> {
    event
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == key)
        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
}

fn read_xlsx(bytes: &[u8]) -> Result<Vec<(String, Vec<(usize, Row)>)>> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes))?;
    let mut shared = Vec::new();
    if let Ok(xml) = archive_text(&mut archive, "xl/sharedStrings.xml") {
        let mut reader = Reader::from_str(&xml);
        let mut value = String::new();
        let mut in_text = false;
        loop {
            match reader.read_event()? {
                Event::Start(e) if e.local_name().as_ref() == b"si" => value.clear(),
                Event::Start(e) if e.local_name().as_ref() == b"t" => in_text = true,
                Event::Text(e) if in_text => {
                    value.push_str(&quick_xml::escape::unescape(std::str::from_utf8(e.as_ref())?)?)
                }
                Event::GeneralRef(e) if in_text => value.push_str(&decode_reference(e.as_ref())?),
                Event::End(e) if e.local_name().as_ref() == b"t" => in_text = false,
                Event::End(e) if e.local_name().as_ref() == b"si" => shared.push(value.clone()),
                Event::Eof => break,
                _ => {}
            }
        }
    }
    let relationships = archive_text(&mut archive, "xl/_rels/workbook.xml.rels")?;
    let mut reader = Reader::from_str(&relationships);
    let mut targets = HashMap::new();
    loop {
        match reader.read_event()? {
            Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == b"Relationship" => {
                if let (Some(id), Some(target)) = (attr(&e, b"Id"), attr(&e, b"Target")) {
                    targets.insert(id, target);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    let workbook = archive_text(&mut archive, "xl/workbook.xml")?;
    let mut reader = Reader::from_str(&workbook);
    let mut sheets = Vec::new();
    loop {
        match reader.read_event()? {
            Event::Empty(e) | Event::Start(e) if e.local_name().as_ref() == b"sheet" => {
                if let (Some(name), Some(id)) = (attr(&e, b"name"), attr(&e, b"r:id")) {
                    let target = targets
                        .get(&id)
                        .ok_or_else(|| anyhow!("Missing sheet relationship"))?;
                    let path = if target.starts_with('/') {
                        target.trim_start_matches('/').to_string()
                    } else {
                        format!("xl/{}", target.trim_start_matches("./"))
                    };
                    let xml = archive_text(&mut archive, &path)?;
                    sheets.push((name, worksheet_rows(&xml, &shared)?));
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(sheets)
}

fn worksheet_rows(xml: &str, shared: &[String]) -> Result<Vec<(usize, Row)>> {
    let mut reader = Reader::from_str(xml);
    let mut rows = Vec::new();
    let mut row = Row::new();
    let mut row_number = 0;
    let mut cell = String::new();
    let mut kind = String::new();
    let mut value = String::new();
    let mut in_value = false;
    let mut formula = false;
    loop {
        match reader.read_event()? {
            Event::Start(e) if e.local_name().as_ref() == b"row" => {
                row.clear();
                row_number = attr(&e, b"r")
                    .and_then(|n| n.parse().ok())
                    .unwrap_or(rows.len() + 1);
            }
            Event::Start(e) if e.local_name().as_ref() == b"c" => {
                cell = attr(&e, b"r").unwrap_or_default();
                kind = attr(&e, b"t").unwrap_or_default();
                value.clear();
                formula = false;
            }
            Event::Start(e) if e.local_name().as_ref() == b"f" => formula = true,
            Event::Start(e) if matches!(e.local_name().as_ref(), b"v" | b"t") => in_value = true,
            Event::Text(e) if in_value => {
                value.push_str(&quick_xml::escape::unescape(std::str::from_utf8(e.as_ref())?)?)
            }
            Event::GeneralRef(e) if in_value => value.push_str(&decode_reference(e.as_ref())?),
            Event::End(e) if matches!(e.local_name().as_ref(), b"v" | b"t") => in_value = false,
            Event::End(e) if e.local_name().as_ref() == b"c" => {
                // Cached formulas are not a person's stated answer.
                let text = if formula {
                    String::new()
                } else if kind == "s" {
                    value
                        .parse::<usize>()
                        .ok()
                        .and_then(|i| shared.get(i))
                        .cloned()
                        .ok_or_else(|| anyhow!("Invalid shared string at {}", cell))?
                } else {
                    value.clone()
                };
                row.insert(
                    cell.chars()
                        .take_while(|c| c.is_ascii_alphabetic())
                        .collect(),
                    text,
                );
            }
            Event::End(e) if e.local_name().as_ref() == b"row" => {
                rows.push((row_number, row.clone()))
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(rows)
}

fn normalize(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
fn field(header: &str) -> Option<String> {
    let h = normalize(header).replace('_', " ");
    let name = match h.as_str() {
        "question" | "questions" | "scenario" | "prompt" => "question",
        "target answer" | "user answer" | "human answer" => "target_answer",
        "target rationale" | "user rationale" | "human rationale" => "target_rationale",
        "model answer" | "ai answer" => "model_answer",
        "model rationale" | "ai rationale" => "model_rationale",
        "model" | "model name" => "model",
        "options" => "options",
        _ if h.starts_with("option ") => return Some(h.replace(' ', "_")),
        _ => return None,
    };
    Some(name.to_string())
}

fn embedded_options(question: &str) -> Vec<Value> {
    let mut options = Vec::new();
    let mut label = String::new();
    let mut text = String::new();
    for line in question.lines() {
        let line = line.trim();
        let bytes = line.as_bytes();
        if bytes.len() > 2
            && bytes[0].is_ascii_uppercase()
            && matches!(bytes[1], b')' | b'.')
            && bytes[2].is_ascii_whitespace()
        {
            if !label.is_empty() {
                options.push(json!({"label":label,"text":text.trim()}));
            }
            label = (bytes[0] as char).to_ascii_lowercase().to_string();
            text = line[2..].trim().to_string();
        } else if !label.is_empty() && !line.is_empty() {
            text.push('\n');
            text.push_str(line);
        }
    }
    if !label.is_empty() {
        options.push(json!({"label":label,"text":text.trim()}));
    }
    options
}

fn build_batch(
    file: &str,
    sheets: Vec<(String, Vec<(usize, Row)>)>,
    mapping: Option<&crate::models::import::ImportTableMapping>,
) -> Result<DocumentImportBatch> {
    if let Some(m) = mapping {
        if m.first_data_row == 0 || !m.columns.contains_key("question") {
            return Err(anyhow!(
                "Column mapping requires question and a one-based first_data_row"
            ));
        }
        let mut seen = std::collections::HashSet::new();
        for (name, col) in &m.columns {
            if field(name).as_deref() != Some(name.as_str())
                || col.is_empty()
                || !col.chars().all(|c| c.is_ascii_alphabetic())
                || !seen.insert(col.to_ascii_uppercase())
            {
                return Err(anyhow!(
                    "Invalid or conflicting column mapping: {} -> {}",
                    name,
                    col
                ));
            }
        }
    }
    let mut cases = BTreeMap::<String, Value>::new();
    for (sheet, rows) in sheets {
        if mapping
            .and_then(|m| m.sheet.as_deref())
            .is_some_and(|wanted| wanted != sheet)
        {
            continue;
        }
        let mut columns: BTreeMap<String, String> = mapping
            .map(|m| {
                m.columns
                    .iter()
                    .map(|(name, col)| (col.to_ascii_uppercase(), name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        for (number, row) in rows {
            let candidate: BTreeMap<String, String> = row
                .iter()
                .filter_map(|(col, value)| field(value).map(|f| (col.clone(), f)))
                .collect();
            if mapping.is_none() && candidate.values().any(|f| f == "question") {
                columns = candidate;
                continue;
            }
            if mapping.is_some_and(|m| number < m.first_data_row) {
                continue;
            }
            if columns.is_empty() || row.values().all(|s| s.trim().is_empty()) {
                continue;
            }
            let mut values = BTreeMap::new();
            let mut cells = BTreeMap::new();
            let mut issues = Vec::<String>::new();
            for (col, name) in &columns {
                let value = row.get(col).cloned().unwrap_or_default();
                if values.insert(name.clone(), value).is_some() {
                    issues.push(format!("duplicate_column_mapping:{}", name));
                }
                cells.insert(name.clone(), format!("{}{}", col, number));
            }
            let question = values.get("question").cloned().unwrap_or_default();
            if question.trim().is_empty() {
                continue;
            }
            let mut options: Vec<Value> = values
                .iter()
                .filter(|(k, v)| {
                    (k.starts_with("option_") || k.as_str() == "options") && !v.trim().is_empty()
                })
                .map(|(k, v)| json!({"label":k.trim_start_matches("option_"),"text":v}))
                .collect();
            if field(&question).as_deref() == Some("question") {
                continue;
            }
            if options.is_empty() {
                options = embedded_options(&question);
            }
            let fingerprint = format!(
                "{}|{}",
                normalize(&question),
                normalize(&serde_json::to_string(&options)?)
            );
            let mut hash = std::collections::hash_map::DefaultHasher::new();
            fingerprint.hash(&mut hash);
            let key = format!("scenario-{:016x}", hash.finish());
            let get = |name: &str| values.get(name).filter(|v| !v.trim().is_empty()).cloned();
            let receipt = json!({"file":file,"sheet":sheet,"row":number,"cells":cells,"values":values,"raw_cells":row});
            let answer = get("target_answer");
            let rationale = get("target_rationale");
            if answer.is_none() {
                issues.push("missing_target_answer".into());
            }
            if options.is_empty() {
                issues.push("missing_options".into());
            }
            if let (Some(answer), Some(rationale)) = (&answer, &rationale) {
                let selected = normalize(answer);
                let reason = normalize(rationale);
                if selected.len() == 1
                    && options.iter().any(|option| {
                        let label = option["label"].as_str().unwrap_or_default();
                        label.len() == 1
                            && label != selected
                            && (reason.contains(&format!("choose {}", label))
                                || reason.contains(&format!("option {}", label)))
                    })
                {
                    issues.push("answer_rationale_alignment_requires_review".into());
                }
            }
            let case = cases.entry(key.clone()).or_insert_with(|| json!({"scenario_key":key,"question":question,"options":options,"target_answer":answer,"target_rationale":rationale,"model_outputs":[],"receipts":[],"issues":[],"split":"development"}));
            for (name, incoming) in [("target_answer", answer), ("target_rationale", rationale)] {
                if let Some(value) = incoming {
                    if let Some(existing) = case[name].as_str() {
                        if normalize(existing) != normalize(&value) {
                            issues.push(format!("conflicting_{}", name));
                        }
                    } else {
                        case[name] = json!(value);
                    }
                }
            }
            if get("model_answer").is_some() || get("model_rationale").is_some() {
                case["model_outputs"].as_array_mut().unwrap().push(json!({"answer":get("model_answer"),"rationale":get("model_rationale"),"model":get("model"),"receipt":receipt.clone()}));
            }
            case["receipts"].as_array_mut().unwrap().push(receipt);
            for issue in issues {
                let list = case["issues"].as_array_mut().unwrap();
                if !list.contains(&json!(issue)) {
                    list.push(json!(issue));
                }
            }
        }
    }
    let mut rationale_cases = HashMap::<String, Vec<String>>::new();
    for (key, case) in &cases {
        for receipt in case["receipts"].as_array().unwrap() {
            if let Some(reason) = receipt["values"]["target_rationale"]
                .as_str()
                .filter(|v| v.split_whitespace().count() >= 8)
            {
                let keys = rationale_cases.entry(normalize(reason)).or_default();
                if !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
    }
    for keys in rationale_cases.values().filter(|keys| keys.len() > 1) {
        for key in keys {
            let issues = cases.get_mut(key).unwrap()["issues"]
                .as_array_mut()
                .unwrap();
            let flag = json!("rationale_reused_across_different_questions");
            if !issues.contains(&flag) {
                issues.push(flag);
            }
        }
    }
    if cases.is_empty() {
        return Err(anyhow!("No decision rows found. Use explicit Question, Option A/Option B, User Answer, User Rationale, Model Answer and Model Rationale column headers; unknown answer columns are never guessed."));
    }
    let items = cases.into_iter().map(|(key, mut case)| {
        if !case["target_answer"].is_null() { case["issues"].as_array_mut().unwrap().retain(|i| i != "missing_target_answer"); }
        let title = format!("Decision: {}", case["question"].as_str().unwrap().chars().take(100).collect::<String>());
        let content = format!("# {}\n\n{}\n\nOptions: {}\n\nTarget answer: {}\n\nTarget rationale: {}\n\nImport issues: {}\n",title,case["question"].as_str().unwrap(),case["options"],case["target_answer"].as_str().unwrap_or("Unanswered"),case["target_rationale"].as_str().unwrap_or("Not supplied"),case["issues"]);
        let metadata = HashMap::from([("source".into(),json!("decision_table")),("source_type".into(),json!("decision_table")),("source_file_name".into(),json!(file)),("created_via".into(),json!("decision_table_import")),("content_kind".into(),json!("decision_case")),("decision_cases".into(),json!([case]))]);
        DocumentImportItem { id:key,title,content,content_kind:"decision_case".into(),suggested_tags:vec!["import".into(),"evidence".into()],metadata }
    }).collect();
    Ok(DocumentImportBatch {
        source_title: file.into(),
        items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_model_rows_keep_one_case_and_exact_receipts() {
        let csv = "Question,Option A,Option B,User Answer,User Rationale,Model Answer,Model\nWhich?,Fast,Safe,Neither,Need more evidence,A,one\nWhich?,Fast,Safe,Neither,Need more evidence,B,two\n";
        let batch = parse("cases.csv", csv.as_bytes(), false).unwrap();
        assert_eq!(batch.items.len(), 1);
        let c = &batch.items[0].metadata["decision_cases"][0];
        assert_eq!(c["target_answer"], "Neither");
        assert_eq!(c["model_outputs"].as_array().unwrap().len(), 2);
        assert_eq!(c["receipts"][1]["cells"]["target_answer"], "D3");
    }
    #[test]
    fn missing_answer_is_not_filled_from_model_and_conflicting_rationale_is_flagged() {
        let csv="Question,Option A,User Answer,User Rationale,Model Answer\nWhy?,Wait,,Because wait,A\nWhy?,Wait,,Because rush,A\n";
        let batch = parse("cases.csv", csv.as_bytes(), false).unwrap();
        let c = &batch.items[0].metadata["decision_cases"][0];
        assert!(c["target_answer"].is_null());
        assert!(c["issues"]
            .as_array()
            .unwrap()
            .contains(&json!("conflicting_target_rationale")));
    }
    #[test]
    fn csv_quotes_newlines_and_combinations_are_preserved() {
        let rows = csv_rows("Question,User Answer\n\"What, next?\nExplain\",A and B\n").unwrap();
        assert_eq!(rows[1].1["A"], "What, next?\nExplain");
        assert_eq!(rows[1].1["B"], "A and B");
    }

    #[test]
    fn embedded_options_and_repeated_custom_headers_keep_the_full_question() {
        let mapping = crate::models::import::ImportTableMapping {
            columns: BTreeMap::from([
                ("question".into(), "A".into()),
                ("target_answer".into(), "B".into()),
            ]),
            first_data_row: 1,
            sheet: None,
        };
        let csv="Questions,Person response\n\"A synthetic scheduling choice.\nA) Leave now\nB) Wait\",Neither\nQuestions,Person response\n\"A synthetic scheduling choice.\nA) Leave now\nB) Wait\",Neither\n";
        let batch =
            parse_with_mapping("synthetic.csv", csv.as_bytes(), false, Some(&mapping)).unwrap();
        assert_eq!(batch.items.len(), 1);
        let case = &batch.items[0].metadata["decision_cases"][0];
        assert_eq!(case["options"].as_array().unwrap().len(), 2);
        assert_eq!(case["options"][1]["text"], "Wait");
        assert_eq!(case["receipts"].as_array().unwrap().len(), 2);
        assert!(case["question"].as_str().unwrap().contains("A) Leave now"));
        assert!(case["issues"].as_array().unwrap().is_empty());
    }
    #[test]
    fn explicit_headerless_mapping_keeps_exact_cells_and_flags_swapped_rationale() {
        let mapping = crate::models::import::ImportTableMapping {
            columns: BTreeMap::from([
                ("question".into(), "C".into()),
                ("option_a".into(), "D".into()),
                ("option_b".into(), "E".into()),
                ("target_answer".into(), "G".into()),
                ("target_rationale".into(), "H".into()),
            ]),
            first_data_row: 1,
            sheet: None,
        };
        let batch = parse_with_mapping(
            "table.csv",
            b"9,Hard,Which?,Fast,Safe,,A,I choose B\n",
            false,
            Some(&mapping),
        )
        .unwrap();
        let case = &batch.items[0].metadata["decision_cases"][0];
        assert_eq!(case["receipts"][0]["cells"]["target_answer"], "G1");
        assert!(case["issues"]
            .as_array()
            .unwrap()
            .contains(&json!("answer_rationale_alignment_requires_review")));
    }
    #[test]
    fn multi_sheet_xlsx_shared_inline_strings_deduplicate_cases() {
        use std::io::Write;
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let sheet = r#"<worksheet><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>Question</t></is></c><c r="B1" t="inlineStr"><is><t>Option A</t></is></c><c r="C1" t="inlineStr"><is><t>User Answer</t></is></c></row><row r="2"><c r="A2" t="s"><v>0</v></c><c r="B2" t="inlineStr"><is><t>Wait</t></is></c><c r="C2" t="inlineStr"><is><t>A and B</t></is></c></row></sheetData></worksheet>"#;
        for (path, xml) in [
            (
                "xl/workbook.xml",
                r#"<workbook xmlns:r="rels"><sheets><sheet name="First" r:id="r1"/><sheet name="Second" r:id="r2"/></sheets></workbook>"#,
            ),
            (
                "xl/_rels/workbook.xml.rels",
                r#"<Relationships><Relationship Id="r1" Target="worksheets/one.xml"/><Relationship Id="r2" Target="worksheets/two.xml"/></Relationships>"#,
            ),
            (
                "xl/sharedStrings.xml",
                r#"<sst><si><t>What &amp; when?</t></si></sst>"#,
            ),
            ("xl/worksheets/one.xml", sheet),
            ("xl/worksheets/two.xml", sheet),
        ] {
            zip.start_file(path, zip::write::FileOptions::default())
                .unwrap();
            zip.write_all(xml.as_bytes()).unwrap();
        }
        let bytes = zip.finish().unwrap().into_inner();
        let batch = parse("synthetic.xlsx", &bytes, true).unwrap();
        assert_eq!(batch.items.len(), 1);
        let case = &batch.items[0].metadata["decision_cases"][0];
        assert_eq!(case["question"], "What & when?");
        assert_eq!(case["receipts"][1]["sheet"], "Second");
        assert_eq!(case["target_answer"], "A and B");
    }
}
