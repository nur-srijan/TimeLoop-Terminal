use serde::{Deserialize, Serialize};
use reqwest::header::{AUTHORIZATION, HeaderMap, HeaderValue, CONTENT_TYPE, USER_AGENT};
use crate::{Storage, EventType};

#[derive(Serialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
}

#[derive(Deserialize)]
struct ChatResponse {
    choices: Vec<Choice>,
}

#[derive(Deserialize)]
struct Choice {
    message: ChatMessageOut,
}

#[derive(Deserialize)]
struct ChatMessageOut {
    content: String,
}

fn build_timeline(storage: &Storage, session_id: &str, max_items: usize) -> crate::Result<String> {
    let mut events = storage.get_events_for_session(session_id)?;
    events.sort_by_key(|e| e.sequence_number);
    let mut lines: Vec<String> = Vec::new();
    for e in events.into_iter().rev().take(max_items).rev() {
        match e.event_type {
            EventType::Command { ref command, ref working_directory, ref exit_code, .. } => {
                lines.push(format!("[cmd] '{}' (dir: {}, exit: {})", command, working_directory, exit_code));
            }
            EventType::FileChange { ref path, ref change_type, .. } => {
                lines.push(format!("[file] {:?} {}", change_type, path));
            }
            EventType::KeyPress { ref key, .. } => {
                lines.push(format!("[key] {}", key));
            }
            EventType::TerminalState { ref screen_size, .. } => {
                lines.push(format!("[term] size {}x{}", screen_size.0, screen_size.1));
            }
            EventType::SessionMetadata { ref name, .. } => {
                lines.push(format!("[session] {}", name));
            }
        }
    }
    Ok(lines.join("\n"))
}

pub async fn summarize_session(session_id: &str, model: &str) -> crate::Result<String> {
    let storage = Storage::new()?;
    let timeline = build_timeline(&storage, session_id, 200)?;
    let prompt = format!("You are an expert assistant. Summarize the following terminal session succinctly with key actions, commands run, files changed, and possible next steps.\n\n{}", timeline);

    let api_key = std::env::var("OPENROUTER_API_KEY")
        .map_err(|_| crate::error::TimeLoopError::Configuration("Missing OPENROUTER_API_KEY".to_string()))?;
    let base = std::env::var("OPENROUTER_BASE_URL").unwrap_or_else(|_| "https://openrouter.ai/api/v1".to_string());

    let url = format!("{}/chat/completions", base.trim_end_matches('/'));

    let mut headers = HeaderMap::new();
    headers.insert(AUTHORIZATION, HeaderValue::from_str(&format!("Bearer {}", api_key)).unwrap());
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers.insert(USER_AGENT, HeaderValue::from_static("timeloop-terminal/ai"));

    let body = ChatRequest {
        model: model.to_string(),
        messages: vec![
            ChatMessage { role: "system".to_string(), content: "You are a concise expert assistant for terminal session summaries.".to_string() },
            ChatMessage { role: "user".to_string(), content: prompt },
        ],
    };

    let client = reqwest::Client::new();
    let resp = client.post(url)
        .headers(headers)
        .json(&body)
        .send()
        .await
        .map_err(|e| crate::error::TimeLoopError::Unknown(e.to_string()))?;

    if !resp.status().is_success() {
        return Err(crate::error::TimeLoopError::Unknown(format!("OpenRouter request failed: {}", resp.status())));
    }

    let parsed: ChatResponse = resp.json().await.map_err(|e| crate::error::TimeLoopError::Unknown(e.to_string()))?;
    let content = parsed.choices.get(0).map(|c| c.message.content.clone()).unwrap_or_else(|| "No response".to_string());
    Ok(content)
}

