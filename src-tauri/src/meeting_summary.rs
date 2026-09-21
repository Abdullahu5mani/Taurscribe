//! Meeting Summarization, Categorization & Action Item Extraction
//!
//! Generates structured executive intelligence from diarized meeting turns:
//! - Descriptive title
//! - Meeting category ("Engineering", "Standup", "Planning", "1-on-1", "Client Sync", "General")
//! - Bulleted executive summary
//! - Action items with identified assignees and status
//!
//! Uses local LLM (Qwen 2.5 / FlowScribe GGUF) when loaded, with an instant,
//! high-accuracy heuristic NLP parser fallback when offline or model is not downloaded.

use crate::diarization::DiarizedTurn;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionItem {
    pub id: String,
    pub task: String,
    pub assignee: String,
    pub status: String, // "todo" | "done"
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MeetingSummaryOutput {
    pub title: String,
    pub category: String,
    pub summary: Vec<String>,
    pub action_items: Vec<ActionItem>,
}

/// Fallback heuristic extractor that parses diarized turns when local LLM is offline
pub fn extract_summary_heuristics(turns: &[DiarizedTurn], default_title: &str) -> MeetingSummaryOutput {
    if turns.is_empty() {
        return MeetingSummaryOutput {
            title: default_title.to_string(),
            category: "General".to_string(),
            summary: vec!["No audio transcript recorded for this session.".to_string()],
            action_items: Vec::new(),
        };
    }

    let mut full_text = String::new();
    let mut speaker_names = std::collections::HashSet::new();

    for t in turns {
        full_text.push_str(&t.text);
        full_text.push(' ');
        if !t.speaker_name.is_empty() {
            speaker_names.insert(t.speaker_name.as_str());
        }
    }

    let lower = full_text.to_lowercase();

    // 1. Categorization
    let category = if lower.contains("bug")
        || lower.contains("architecture")
        || lower.contains("database")
        || lower.contains("backend")
        || lower.contains("frontend")
        || lower.contains("deploy")
        || lower.contains("rust")
        || lower.contains("api")
    {
        "Engineering".to_string()
    } else if lower.contains("standup")
        || lower.contains("yesterday")
        || lower.contains("blocker")
    {
        "Standup".to_string()
    } else if lower.contains("roadmap")
        || lower.contains("sprint")
        || lower.contains("q3")
        || lower.contains("q4")
        || lower.contains("planning")
        || lower.contains("milestone")
    {
        "Planning".to_string()
    } else if lower.contains("client")
        || lower.contains("contract")
        || lower.contains("demo")
        || lower.contains("customer")
    {
        "Client Sync".to_string()
    } else if speaker_names.len() == 2 {
        "1-on-1".to_string()
    } else {
        "General".to_string()
    };

    // 2. Title generation
    let title = if !default_title.is_empty() && default_title != "Meeting" && default_title != "Call" {
        default_title.to_string()
    } else if category == "Engineering" {
        "Engineering & Architecture Review".to_string()
    } else if category == "Standup" {
        "Team Daily Standup".to_string()
    } else if category == "Planning" {
        "Sprint & Product Roadmap Planning".to_string()
    } else if category == "Client Sync" {
        "Client Project Sync".to_string()
    } else if speaker_names.len() >= 2 {
        let names: Vec<&str> = speaker_names.into_iter().collect();
        format!("Discussion with {}", names.join(" & "))
    } else {
        "Recorded Discussion".to_string()
    };

    // 3. Action item detection
    let mut action_items = Vec::new();
    let action_triggers = [
        ("i will ", 0),
        ("i'll ", 0),
        ("let's ", 1),
        ("we need to ", 1),
        ("please ", 2),
        ("can you ", 2),
        ("make sure to ", 2),
        ("follow up on ", 1),
        ("action item: ", 1),
        ("todo: ", 1),
    ];

    let mut action_idx = 1;
    for turn in turns {
        let _turn_lower = turn.text.to_lowercase();
        let sentences: Vec<&str> = turn.text.split(['.', '?', '!', '\n']).collect();

        for sent in sentences {
            let sent_trim = sent.trim();
            if sent_trim.len() < 10 {
                continue;
            }
            let sent_lower = sent_trim.to_lowercase();

            for (trigger, target_type) in &action_triggers {
                if let Some(pos) = sent_lower.find(trigger) {
                    let task_text = sent_trim[pos..].trim();
                    let assignee = match *target_type {
                        0 => turn.speaker_name.clone(), // Speaker committed to it
                        1 => "Team".to_string(),
                        2 => {
                            if turn.speaker_name == "You" {
                                "Remote".to_string()
                            } else {
                                "You".to_string()
                            }
                        }
                        _ => "Unassigned".to_string(),
                    };

                    action_items.push(ActionItem {
                        id: format!("action_{}", action_idx),
                        task: clean_action_task(task_text),
                        assignee,
                        status: "todo".to_string(),
                    });
                    action_idx += 1;
                    break;
                }
            }
        }
    }

    // No placeholder item when nothing was found: the review modal lists these
    // as "Action items detected", and a stock task nobody said was misleading.

    // 4. Executive summary points
    let mut summary = Vec::new();
    let mut current_speaker = "";
    let mut speaker_points = Vec::new();

    for t in turns {
        if t.text.trim().len() > 15 {
            if t.speaker_name != current_speaker {
                current_speaker = &t.speaker_name;
                speaker_points.push(format!("{}: {}", current_speaker, t.text.trim()));
            } else if let Some(last) = speaker_points.last_mut() {
                last.push(' ');
                last.push_str(t.text.trim());
            }
        }
    }

    if speaker_points.is_empty() {
        summary.push(format!("Meeting concluded with {} turns recorded.", turns.len()));
    } else {
        for pt in speaker_points.into_iter().take(4) {
            let truncated = crate::utils::truncate_utf8_with_ellipsis(&pt, 160);
            summary.push(truncated);
        }
    }

    MeetingSummaryOutput {
        title,
        category,
        summary,
        action_items,
    }
}

fn clean_action_task(raw: &str) -> String {
    let task = raw.trim();
    // Capitalize first letter
    let mut chars = task.chars();
    match chars.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// Parses an LLM JSON completion into `MeetingSummaryOutput`
pub fn parse_llm_summary_json(json_str: &str, fallback: MeetingSummaryOutput) -> MeetingSummaryOutput {
    // Attempt direct parse or extract JSON block {...}
    let candidate = if let (Some(s), Some(e)) = (json_str.find('{'), json_str.rfind('}')) {
        &json_str[s..=e]
    } else {
        json_str
    };

    #[derive(Deserialize)]
    struct LlmResponse {
        title: Option<String>,
        category: Option<String>,
        summary: Option<Vec<String>>,
        action_items: Option<Vec<LlmActionItem>>,
    }

    #[derive(Deserialize)]
    struct LlmActionItem {
        task: String,
        assignee: Option<String>,
        status: Option<String>,
    }

    if let Ok(parsed) = serde_json::from_str::<LlmResponse>(candidate) {
        let title = parsed.title.filter(|t| !t.trim().is_empty()).unwrap_or(fallback.title);
        let category = parsed.category.filter(|c| !c.trim().is_empty()).unwrap_or(fallback.category);
        let summary = parsed.summary.filter(|s| !s.is_empty()).unwrap_or(fallback.summary);

        let action_items = if let Some(items) = parsed.action_items {
            items
                .into_iter()
                .enumerate()
                .map(|(i, item)| ActionItem {
                    id: format!("action_{}", i + 1),
                    task: item.task,
                    assignee: item.assignee.unwrap_or_else(|| "You".to_string()),
                    status: item.status.unwrap_or_else(|| "todo".to_string()),
                })
                .collect()
        } else {
            fallback.action_items
        };

        MeetingSummaryOutput {
            title,
            category,
            summary,
            action_items,
        }
    } else {
        fallback
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_summary_heuristics_engineering() {
        let turns = vec![
            DiarizedTurn {
                speaker_id: "speaker_you".to_string(),
                speaker_name: "You".to_string(),
                start_ms: 0,
                end_ms: 2000,
                channel: 0,
                text: "Let's review the new database architecture and Rust backend.".to_string(),
                snippet_path: None,
                candidate_snippets: Vec::new(),
                current_snippet_idx: 0,
            },
            DiarizedTurn {
                speaker_id: "speaker_remote_1".to_string(),
                speaker_name: "Alice".to_string(),
                start_ms: 2500,
                end_ms: 5000,
                channel: 1,
                text: "I will implement the SQLite migration for meeting turns.".to_string(),
                snippet_path: None,
                candidate_snippets: Vec::new(),
                current_snippet_idx: 0,
            },
        ];

        let out = extract_summary_heuristics(&turns, "Architecture Call");
        assert_eq!(out.category, "Engineering");
        assert_eq!(out.title, "Architecture Call");
        assert_eq!(out.action_items.len(), 2);
        assert_eq!(out.action_items[0].assignee, "Team");
        assert_eq!(out.action_items[1].assignee, "Alice");
        assert!(out.action_items[1].task.contains("implement the SQLite"));
    }

    #[test]
    fn test_parse_llm_summary_json() {
        let json_input = r#"{
            "title": "Google Meet Sprint Sync",
            "category": "Standup",
            "summary": ["Reviewed PRs", "Fixed speaker crackle bug"],
            "action_items": [
                { "task": "Ship v0.2.0 build", "assignee": "Bob", "status": "todo" }
            ]
        }"#;

        let fallback = MeetingSummaryOutput {
            title: "Default".to_string(),
            category: "General".to_string(),
            summary: vec![],
            action_items: vec![],
        };

        let parsed = parse_llm_summary_json(json_input, fallback);
        assert_eq!(parsed.title, "Google Meet Sprint Sync");
        assert_eq!(parsed.category, "Standup");
        assert_eq!(parsed.summary.len(), 2);
        assert_eq!(parsed.action_items[0].assignee, "Bob");
    }
}
