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

    // 1. Categorization, by whole words and their plural/verb forms: "trust"
    // is not "rust", "rapid capital" is not "api", "democracy" is not "demo".
    let words: std::collections::HashSet<&str> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let mentions = |keywords: &[&str]| {
        words.iter().any(|w| {
            keywords.iter().any(|k| w.strip_prefix(k).is_some_and(|rest| ["", "s", "es", "ed", "ing", "ment", "ments"].contains(&rest)))
        })
    };
    let category = if mentions(&["bug", "debug", "debugging", "architecture", "database", "backend", "frontend", "deploy", "rust", "api"]) {
        "Engineering".to_string()
    } else if mentions(&["standup", "yesterday", "blocker"]) {
        "Standup".to_string()
    } else if mentions(&["roadmap", "sprint", "q3", "q4", "planning", "milestone"]) {
        "Planning".to_string()
    } else if mentions(&["client", "contract", "demo", "customer"]) {
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
            // The triggers are ASCII. Full lowercasing changes the byte length of
            // some letters ("İ", the Kelvin sign), and `pos` must index `sent_trim`.
            let sent_lower = sent_trim.to_ascii_lowercase();

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

    fn turn(speaker: &str, text: &str) -> DiarizedTurn {
        DiarizedTurn {
            speaker_id: format!("speaker_{speaker}"),
            speaker_name: speaker.to_string(),
            start_ms: 0,
            end_ms: 2000,
            channel: 1,
            text: text.to_string(),
            snippet_path: None,
            candidate_snippets: Vec::new(),
            current_snippet_idx: 0,
        }
    }

    #[test]
    fn action_items_start_at_the_trigger_after_non_ascii_letters() {
        // Lowercasing "İ" adds a byte, so a position found in the lowercased
        // sentence is past the trigger in the original.
        let out = extract_summary_heuristics(&[turn("Alice", "İrem, can you share the report today.")], "Call");
        assert_eq!(out.action_items.len(), 1);
        assert_eq!(out.action_items[0].task, "Can you share the report today");
        // Lowercasing the Kelvin sign removes two bytes: the slice used to start
        // inside "é" and panic, taking the whole meeting save with it.
        let out = extract_summary_heuristics(&[turn("Alice", "\u{212A}é let's go over the slides")], "Call");
        assert_eq!(out.action_items[0].task, "Let's go over the slides");
    }

    #[test]
    fn categories_match_whole_words_not_parts_of_words() {
        let client = extract_summary_heuristics(&[turn("Alice", "We trust the client with the contract renewal.")], "Call");
        assert_eq!(client.category, "Client Sync");
        let planning = extract_summary_heuristics(&[turn("Alice", "Rapid capital planning for the next milestone.")], "Call");
        assert_eq!(planning.category, "Planning");
        let engineering = extract_summary_heuristics(&[turn("Alice", "The API returns a bug, so we debug the backend.")], "Call");
        assert_eq!(engineering.category, "Engineering");
    }

    #[test]
    fn heuristics_on_empty_meeting() {
        let out = extract_summary_heuristics(&[], "Call");
        assert_eq!(out.title, "Call");
        assert_eq!(out.category, "General");
        assert!(out.action_items.is_empty());
        assert_eq!(out.summary.len(), 1);
    }
}
