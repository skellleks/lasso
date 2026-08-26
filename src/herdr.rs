//! Client for the herdr socket API and CLI.

use std::path::PathBuf;
use std::process::Command;

use anyhow::{bail, Context, Result};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct Agent {
    pub pane_id: String,
    pub agent: String,
    pub agent_status: String,
    pub cwd: String,
    #[serde(default)]
    pub terminal_title_stripped: String,
    #[serde(default)]
    pub workspace_id: String,
}

impl Agent {
    /// Stable key for the comment store.
    pub fn key(&self) -> String {
        format!("{}|{}", self.pane_id, self.cwd)
    }

    /// Short label for the sidebar: last path component of cwd.
    pub fn label(&self) -> &str {
        self.cwd.rsplit('/').next().filter(|s| !s.is_empty()).unwrap_or(&self.cwd)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentStatusEvent {
    pub pane_id: String,
    pub agent_status: String,
}

pub fn parse_agent_list(json: &str) -> Result<Vec<Agent>> {
    #[derive(Deserialize)]
    struct Envelope {
        result: Option<AgentsResult>,
        error: Option<serde_json::Value>,
    }
    #[derive(Deserialize)]
    struct AgentsResult {
        agents: Vec<Agent>,
    }
    let env: Envelope = serde_json::from_str(json).context("bad agent list json")?;
    if let Some(err) = env.error {
        bail!("herdr error: {err}");
    }
    Ok(env.result.context("agent list response has no result")?.agents)
}

/// One NDJSON request line subscribing to status changes for the given panes.
pub fn subscribe_request_line(req_id: &str, pane_ids: &[&str]) -> String {
    let subs: Vec<serde_json::Value> = pane_ids
        .iter()
        .map(|p| serde_json::json!({"type": "pane.agent_status_changed", "pane_id": p}))
        .collect();
    let req = serde_json::json!({
        "id": req_id,
        "method": "events.subscribe",
        "params": {"subscriptions": subs},
    });
    format!("{req}\n")
}

/// Parse one NDJSON line from the socket; Some for agent status change events.
pub fn parse_event_line(line: &str) -> Option<AgentStatusEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("event")?.as_str()? != "pane.agent_status_changed" {
        return None;
    }
    let data = v.get("data")?;
    Some(AgentStatusEvent {
        pane_id: data.get("pane_id")?.as_str()?.to_string(),
        agent_status: data.get("agent_status")?.as_str()?.to_string(),
    })
}

/// (focused pane id, workspace id) from HERDR_PLUGIN_CONTEXT_JSON.
pub fn parse_plugin_context(json: &str) -> (Option<String>, Option<String>) {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return (None, None);
    };
    // flat keys (what herdr actually sends), with the docs' nested shape as fallback
    let flat = |key: &str| v.get(key).and_then(|s| s.as_str()).map(str::to_string);
    let nested = |obj: &str| {
        v.get(obj)
            .and_then(|o| o.get("id"))
            .and_then(|s| s.as_str())
            .map(str::to_string)
    };
    (
        flat("focused_pane_id").or_else(|| nested("focused_pane")),
        flat("workspace_id").or_else(|| nested("workspace")),
    )
}

/// Which agent pane a new lasso window should pin to: the invoking pane if it
/// hosts an agent, else the first agent of the invoking workspace, else the
/// first agent at all.
pub fn resolve_pin(agents: &[Agent], pane: Option<&str>, ws: Option<&str>) -> Option<String> {
    pane.and_then(|p| agents.iter().find(|a| a.pane_id == p))
        .or_else(|| ws.and_then(|w| agents.iter().find(|a| a.workspace_id == w)))
        .or_else(|| agents.first())
        .map(|a| a.pane_id.clone())
}

/// Pane id from a `plugin pane open` response.
pub fn parse_opened_pane_id(json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(json)
        .ok()?
        .pointer("/result/plugin_pane/pane/pane_id")?
        .as_str()
        .map(str::to_string)
}

/// Pane ids from a `herdr pane list` response whose label matches.
pub fn parse_pane_ids_by_label(json: &str, label: &str) -> Vec<String> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    v.pointer("/result/panes")
        .and_then(|p| p.as_array())
        .map(|panes| {
            panes
                .iter()
                .filter(|p| p.get("label").and_then(|l| l.as_str()) == Some(label))
                .filter_map(|p| p.get("pane_id").and_then(|id| id.as_str()).map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Path to the herdr binary: HERDR_BIN_PATH, or "herdr" from PATH.
pub fn herdr_bin() -> PathBuf {
    std::env::var_os("HERDR_BIN_PATH").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("herdr"))
}

pub fn agent_list() -> Result<Vec<Agent>> {
    let out = Command::new(herdr_bin())
        .args(["agent", "list"])
        .output()
        .context("failed to run herdr agent list")?;
    if !out.status.success() {
        bail!("herdr agent list failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    parse_agent_list(&String::from_utf8_lossy(&out.stdout))
}

pub fn agent_prompt(pane_id: &str, text: &str) -> Result<()> {
    let out = Command::new(herdr_bin())
        .args(["agent", "prompt", pane_id, text])
        .output()
        .context("failed to run herdr agent prompt")?;
    if !out.status.success() {
        bail!("herdr agent prompt failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const LIST: &str = r#"{"id":"cli:agent:list","result":{"agents":[
        {"agent":"claude","agent_status":"idle","cwd":"/repo/api",
         "pane_id":"w1:p1","tab_id":"w1:t1","workspace_id":"w1",
         "terminal_title_stripped":"fix login"},
        {"agent":"codex","agent_status":"working","cwd":"/repo/web",
         "pane_id":"w1:p2","tab_id":"w1:t2","workspace_id":"w1"}
    ]}}"#;

    #[test]
    fn parses_agent_list_cli_output() {
        let agents = parse_agent_list(LIST).unwrap();
        assert_eq!(agents.len(), 2);
        assert_eq!(agents[0].pane_id, "w1:p1");
        assert_eq!(agents[0].agent_status, "idle");
        assert_eq!(agents[0].cwd, "/repo/api");
        assert_eq!(agents[0].terminal_title_stripped, "fix login");
        assert_eq!(agents[1].agent, "codex");
    }

    #[test]
    fn agent_list_error_response_is_err() {
        assert!(parse_agent_list(r#"{"id":"x","error":{"message":"boom"}}"#).is_err());
        assert!(parse_agent_list("not json").is_err());
    }

    #[test]
    fn agent_label_is_cwd_basename() {
        let a = parse_agent_list(LIST).unwrap();
        assert_eq!(a[0].label(), "api");
        assert_eq!(a[0].key(), "w1:p1|/repo/api");
    }

    #[test]
    fn subscribe_line_covers_all_panes() {
        let line = subscribe_request_line("sub-1", &["w1:p1", "w1:p2"]);
        assert!(line.ends_with('\n'));
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(v["id"], "sub-1");
        assert_eq!(v["method"], "events.subscribe");
        let subs = v["params"]["subscriptions"].as_array().unwrap();
        assert_eq!(subs.len(), 2);
        assert_eq!(subs[0]["type"], "pane.agent_status_changed");
        assert_eq!(subs[0]["pane_id"], "w1:p1");
        assert_eq!(subs[1]["pane_id"], "w1:p2");
    }

    #[test]
    fn parses_status_change_event() {
        let line = r#"{"event":"pane.agent_status_changed","data":{"pane_id":"w1:p2","workspace_id":"w1","agent_status":"idle","agent":"codex"}}"#;
        let ev = parse_event_line(line).unwrap();
        assert_eq!(ev.pane_id, "w1:p2");
        assert_eq!(ev.agent_status, "idle");
    }

    #[test]
    fn parses_plugin_context() {
        // the flat shape herdr 0.7.5 actually sends
        let json = r#"{"workspace_id":"w2","workspace_label":"Navetix","tab_id":"w2:t1","focused_pane_id":"w2:pH","focused_pane_agent":"claude","invocation_source":"api"}"#;
        assert_eq!(
            parse_plugin_context(json),
            (Some("w2:pH".to_string()), Some("w2".to_string()))
        );
        // the nested shape from the docs example, just in case
        let json = r#"{"workspace":{"id":"w3","name":"Local"},"focused_pane":{"id":"w3:p1"}}"#;
        assert_eq!(
            parse_plugin_context(json),
            (Some("w3:p1".to_string()), Some("w3".to_string()))
        );
        assert_eq!(parse_plugin_context("{}"), (None, None));
        assert_eq!(parse_plugin_context("garbage"), (None, None));
    }

    #[test]
    fn resolve_pin_prefers_pane_then_workspace() {
        let mk = |pane: &str, ws: &str| -> Agent {
            serde_json::from_value(serde_json::json!({
                "pane_id": pane, "agent": "claude", "agent_status": "idle",
                "cwd": "/r", "workspace_id": ws
            }))
            .unwrap()
        };
        let agents = vec![mk("w1:p1", "w1"), mk("w2:p5", "w2")];
        assert_eq!(resolve_pin(&agents, Some("w2:p5"), None), Some("w2:p5".into()));
        // pane not an agent → first agent of the workspace
        assert_eq!(resolve_pin(&agents, Some("w2:p9"), Some("w2")), Some("w2:p5".into()));
        // nothing matches → first agent overall
        assert_eq!(resolve_pin(&agents, Some("w9:p9"), Some("w9")), Some("w1:p1".into()));
        assert_eq!(resolve_pin(&[], Some("w1:p1"), None), None);
    }

    #[test]
    fn parses_opened_pane_id() {
        let json = r#"{"id":"x","result":{"plugin_pane":{"entrypoint":"review","pane":{"pane_id":"w3:pB","workspace_id":"w3"},"plugin_id":"lasso"},"type":"plugin_pane_opened"}}"#;
        assert_eq!(parse_opened_pane_id(json), Some("w3:pB".to_string()));
        assert_eq!(parse_opened_pane_id("nope"), None);
    }

    #[test]
    fn finds_panes_by_label() {
        let json = r#"{"id":"x","result":{"panes":[
            {"pane_id":"w1:p1","label":null},
            {"pane_id":"w2:pK","label":"Lasso review"},
            {"pane_id":"w3:p9","label":"Lasso review"},
            {"pane_id":"w3:p2","label":"other"}
        ]}}"#;
        assert_eq!(parse_pane_ids_by_label(json, "Lasso review"), vec!["w2:pK", "w3:p9"]);
        assert!(parse_pane_ids_by_label("garbage", "Lasso review").is_empty());
    }

    #[test]
    fn ignores_other_lines() {
        assert!(parse_event_line(r#"{"id":"sub-1","result":{"ok":true}}"#).is_none());
        assert!(parse_event_line(r#"{"event":"pane.closed","data":{"pane_id":"w1:p9"}}"#).is_none());
        assert!(parse_event_line("garbage").is_none());
    }
}
