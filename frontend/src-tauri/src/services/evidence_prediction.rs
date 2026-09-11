//! Prospective comparisons: same questions/model/settings, frozen development evidence.
use super::{
    atomic_io::write_atomic,
    evidence::*,
    evidence_bridge::{self, EvidenceLocation},
};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

pub const PILOT_MODEL: &str = "qwen3.6:27b";
mod run;
pub use run::predict;
const CONDITIONS: [&str; 3] = ["no_evidence", "personal_evidence", "goal_paths"];

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct PredictionRequest {
    pub id: Option<String>,
    pub batch_id: Option<String>,
    pub domain: Domain,
    pub situation: String,
    pub options: Vec<String>,
    pub clarifications: Vec<Clarification>,
    pub validation: bool,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Clarification {
    pub question: String,
    pub answer: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Branch {
    pub condition: String,
    pub action: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Forecast {
    pub clarification_topics: Vec<String>,
    pub proposed_action: Option<String>,
    pub conditional_branches: Vec<Branch>,
    pub questions: Vec<String>,
    pub assumptions: Vec<String>,
    pub evidence_ids: Vec<String>,
    pub insufficient_evidence: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Comparison {
    #[serde(default)]
    pub request_payload: Value,
    pub condition: String,
    pub stage: String,
    pub status: String,
    pub forecast: Option<Forecast>,
    pub raw_response: Option<String>,
    pub error: Option<String>,
    pub context: ContextPacket,
    pub recorded_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionRecord {
    pub id: String,
    pub batch_id: String,
    pub request: PredictionRequest,
    pub comparisons: Vec<Comparison>,
    pub human_choice: Option<String>,
    pub human_rationale: Option<String>,
    pub adjudications: std::collections::BTreeMap<String, String>,
    pub recorded_at: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Batch {
    id: String,
    model: String,
    model_digest: String,
    settings: Value,
    evidence: EvidenceSnapshot,
    created_at: String,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct Ledger {
    batches: Vec<Batch>,
    records: Vec<PredictionRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ChoiceRequest {
    pub prediction_id: String,
    pub choice: String,
    pub rationale: Option<String>,
    /// Human judgement only. Keys are condition:stage; values agree/disagree/ambiguous.
    pub adjudications: std::collections::BTreeMap<String, String>,
    pub adjudication: Option<String>,
}

fn ledger_transaction<T>(
    loc: &EvidenceLocation,
    f: impl FnOnce(&mut Ledger) -> Result<T>,
) -> Result<T> {
    evidence_bridge::transaction(loc, |_| {
        let path = loc.root.join("predictions.json");
        let mut ledger: Ledger = if path.exists() {
            serde_json::from_slice(&std::fs::read(&path)?)?
        } else {
            Ledger::default()
        };
        let result = f(&mut ledger)?;
        write_atomic(&path, &serde_json::to_vec_pretty(&ledger)?)?;
        Ok(result)
    })
}

fn public_record(record: &PredictionRecord) -> Value {
    let sealed = record.request.validation && record.human_choice.is_none();
    let latest = record
        .comparisons
        .iter()
        .rev()
        .find(|c| c.condition == "goal_paths");
    let forecast = latest.and_then(|c| c.forecast.as_ref());
    let mut value = json!({"id":record.id,"batch_id":record.batch_id,"validation":record.request.validation,
        "sealed":sealed,"status":latest.map(|c|c.status.as_str()).unwrap_or("pending"),
        "questions":forecast.map(|f|if sealed {neutral_questions(&f.clarification_topics)}else{f.questions.iter().take(2).cloned().collect::<Vec<_>>()}).unwrap_or_default(),
        "human_choice":record.human_choice,"human_rationale":record.human_rationale,
        "request":record.request,"recorded_at":record.recorded_at});
    // This is the only IPC/export projection. Never serialize sealed raw responses or context.
    if !sealed {
        value["proposed_action"] = json!(forecast.and_then(|f| f.proposed_action.clone()));
        value["conditional_branches"] = json!(forecast
            .map(|f| f.conditional_branches.clone())
            .unwrap_or_default());
        value["assumptions"] = json!(forecast.map(|f| f.assumptions.clone()).unwrap_or_default());
        value["evidence_ids"] = json!(forecast.map(|f| f.evidence_ids.clone()).unwrap_or_default());
        value["goal_revisions"] = json!(latest
            .map(|c| c
                .context
                .goals
                .iter()
                .map(|g| json!({"goal_id":g.input.id,"revision":g.revision}))
                .collect::<Vec<_>>())
            .unwrap_or_default());
        value["comparisons"] = json!(record.comparisons);
        value["adjudications"] = json!(record.adjudications);
    }
    value
}

fn neutral_questions(topics:&[String])->Vec<String>{
    let mut questions=Vec::new();
    for topic in topics {
        let question=match topic.as_str(){
            "timeframe"=>"What deadline or timeframe applies to this decision?",
            "budget"=>"What budget or resource limit applies?",
            "success_metric"=>"What concrete result would count as success, and how would you measure it?",
            "competing_goals"=>"Which other goals need to be considered here?",
            "constraints"=>"What costs or constraints would make an option unacceptable?",
            "alternatives"=>"Are combined choices or alternatives beyond the listed options available?",
            _=>continue,
        };
        if !questions.iter().any(|q|q==question){questions.push(question.to_string());}
        if questions.len()==2{break;}
    }
    questions
}

pub fn record_choice(loc: &EvidenceLocation, request: ChoiceRequest) -> Result<Value> {
    ensure!(
        !request.choice.trim().is_empty(),
        "Record your actual choice, including combined or outside choices"
    );
    ledger_transaction(loc, |ledger| {
        let r = ledger
            .records
            .iter_mut()
            .find(|r| r.id == request.prediction_id)
            .context("Prediction not found in this vault")?;
        ensure!(
            r.comparisons.len() >= 3 && !r.comparisons.iter().any(|c| c.status == "pending"),
            "Prediction is still running"
        );
        if let Some(choice) = &r.human_choice {
            ensure!(
                choice == &request.choice,
                "Recorded choice is immutable; adjudicate equivalence separately"
            );
        }
        r.human_choice = Some(request.choice);
        r.human_rationale = request.rationale;
        for (key, value) in request.adjudications {
            ensure!(
                ["agree", "disagree", "ambiguous"].contains(&value.as_str()),
                "Unknown adjudication"
            );
            ensure!(
                r.comparisons
                    .iter()
                    .any(|c| format!("{}:{}", c.condition, c.stage) == key),
                "Unknown comparison"
            );
            r.adjudications.insert(key, value);
        }
        Ok(public_record(r))
    })
}

fn parse_forecast(raw: &str) -> Result<Forecast> {
    let clean = raw
        .trim()
        .strip_prefix("```json")
        .or_else(|| raw.trim().strip_prefix("```"))
        .map(|s| s.trim().trim_end_matches("```").trim())
        .unwrap_or(raw.trim());
    let value: Value = serde_json::from_str(clean).context("Prediction is not valid JSON")?;
    ensure!(
        value.is_object() && value.get("insufficient_evidence").is_some(),
        "Prediction schema missing insufficient_evidence"
    );
    let mut f: Forecast = serde_json::from_value(value)?;
    ensure!(
        f.insufficient_evidence
            || f.proposed_action
                .as_ref()
                .is_some_and(|s| !s.trim().is_empty())
            || !f.conditional_branches.is_empty(),
        "No action, conditional branch, or abstention"
    );
    f.questions.truncate(2);
    ensure!(f.conditional_branches.iter().all(|b|!b.condition.trim().is_empty() && !b.action.trim().is_empty()),"Empty conditional branch");
    Ok(f)
}

async fn model_digest(url: &str) -> Result<String> {
    let tags: Value = reqwest::Client::new()
        .get(format!("{}/api/tags", url.trim_end_matches('/')))
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    tags["models"]
        .as_array()
        .and_then(|models| {
            models
                .iter()
                .find(|m| m["name"].as_str() == Some(PILOT_MODEL))
        })
        .and_then(|m| m["digest"].as_str())
        .map(str::to_string)
        .context("Install the configured qwen3.6:27b locally before starting the pilot")
}

pub fn list_predictions(loc: &EvidenceLocation) -> Result<Value> {
    ledger_transaction(loc, |ledger| {
        Ok(
            json!({"records":ledger.records.iter().map(public_record).collect::<Vec<_>>(),
        "batches":ledger.batches.iter().map(|b|json!({"id":b.id,"model":b.model,"model_digest":b.model_digest,"settings":b.settings,"created_at":b.created_at})).collect::<Vec<_>>() }),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sealed_projection_has_no_action_branches_raw_or_context() {
        let r = PredictionRecord {
            id: "id".into(),
            batch_id: "batch".into(),
            request: PredictionRequest {
                validation: true,
                ..Default::default()
            },
            comparisons: vec![Comparison {
                request_payload: Value::Null,
                condition: "goal_paths".into(),
                stage: "before_clarification".into(),
                status: "completed".into(),
                forecast: Some(Forecast {
                    questions: vec!["SECRET_QUESTION".into()],
                    clarification_topics: vec!["budget".into()],
                    proposed_action: Some("SECRET_ACTION".into()),
                    conditional_branches: vec![Branch {
                        condition: "SECRET_CONDITION".into(),
                        action: "SECRET_BRANCH".into(),
                    }],
                    ..Default::default()
                }),
                raw_response: Some("SECRET_RAW".into()),
                error: None,
                context: ContextPacket::default(),
                recorded_at: String::new(),
            }],
            human_choice: None,
            human_rationale: None,
            adjudications: Default::default(),
            recorded_at: String::new(),
        };
        let public = public_record(&r).to_string();
        assert!(!public.contains("SECRET"));
        assert!(!public.contains("comparisons"));
        let mut revealed = r;
        revealed.human_choice = Some("Both".into());
        assert!(public_record(&revealed)
            .to_string()
            .contains("SECRET_ACTION"));
    }
    #[test]
    fn parsing_preserves_outside_choices_and_rejects_empty_objects() {
        assert!(parse_forecast("{}").is_err());
        let f=parse_forecast(r#"{"proposed_action":"Both, but later","insufficient_evidence":false,"questions":["a","b","c"]}"#).unwrap();
        assert_eq!(f.proposed_action.as_deref(), Some("Both, but later"));
        assert_eq!(f.questions.len(), 2);
    }
}
