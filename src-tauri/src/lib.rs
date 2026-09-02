//! HitAI 데스크톱 앱 백엔드. 실제 동작은 모두 hitai-core에 있다.

mod window;

use hitai_core::link::{self, Tool};
use hitai_core::sessions::{self, Session};
use hitai_core::{Config, State, MAX_HP};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct LogEntry {
    at: String,
    reason: String,
    reply: String,
    rule: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ToolView {
    id: &'static str,
    label: &'static str,
    installed: bool,
    active: bool,
}

#[derive(Debug, Serialize)]
pub struct Snapshot {
    hp: i32,
    max_hp: i32,
    total_hits: u64,
    rules: Vec<String>,
    log: Vec<LogEntry>,
    has_key: bool,
    tools: Vec<ToolView>,
    sessions: Vec<Session>,
}

#[derive(Debug, Serialize)]
pub struct HitResult {
    reply: String,
    rule: Option<String>,
    /// 세션에 어떻게 전달되었는지 사람이 읽는 한 줄.
    delivery: String,
    delivered: bool,
    snapshot: Snapshot,
}

fn snapshot_of(state: &State) -> Snapshot {
    let log = state
        .hits
        .iter()
        .rev()
        .take(50)
        .map(|h| LogEntry {
            at: h
                .at
                .with_timezone(&chrono::Local)
                .format("%m/%d %H:%M")
                .to_string(),
            reason: h.reason.clone(),
            reply: h.reply.clone(),
            rule: h.rule.clone(),
        })
        .collect();

    Snapshot {
        hp: state.hp,
        max_hp: MAX_HP,
        total_hits: state.total_hits,
        rules: hitai_core::rule_lines(),
        log,
        has_key: !Config::load().api_key.trim().is_empty(),
        tools: link::status()
            .into_iter()
            .map(|s| ToolView {
                id: s.id,
                label: s.label,
                installed: s.installed,
                active: s.active,
            })
            .collect(),
        sessions: sessions::list(),
    }
}

#[tauri::command]
fn get_snapshot() -> Snapshot {
    snapshot_of(&State::load())
}

/// 세션 목록만 돌려준다. 화면이 짧은 주기로 새로 고칠 때 쓴다.
/// 파일 수정 시각이 그대로인 세션은 캐시에서 나오므로 비용이 거의 없다.
#[tauri::command]
fn list_sessions() -> Vec<Session> {
    sessions::list()
}

#[tauri::command]
async fn hit(reason: String, target: Option<String>) -> Result<HitResult, String> {
    let outcome = hitai_core::hit(&reason, target).await?;
    Ok(HitResult {
        reply: outcome.reply,
        rule: outcome.rule,
        delivery: outcome.delivery.describe(),
        delivered: outcome.delivery == hitai_core::Delivery::Sent,
        snapshot: snapshot_of(&outcome.state),
    })
}

/// 규칙 한 줄을 사용자가 직접 고친다.
#[tauri::command]
fn edit_rule(old: String, new: String) -> Result<Snapshot, String> {
    hitai_core::edit_rule(&old, &new).map_err(|e| e.to_string())?;
    Ok(snapshot_of(&State::load()))
}

/// 규칙 목록 전체를 바꾼다. 순서 변경이나 붙여넣기 편집에 쓴다.
#[tauri::command]
fn set_rules(rules: Vec<String>) -> Result<Snapshot, String> {
    hitai_core::set_rules(&rules).map_err(|e| e.to_string())?;
    Ok(snapshot_of(&State::load()))
}

/// AI가 규칙을 합치고 문장을 다듬는다.
#[tauri::command]
async fn tidy_rules() -> Result<Snapshot, String> {
    let key = Config::load().api_key.trim().to_string();
    if key.is_empty() {
        return Err("API 키가 필요합니다. 설정에서 저장해 주세요.".into());
    }
    let before = hitai_core::rule_lines();
    if before.is_empty() {
        return Err("정리할 규칙이 없습니다.".into());
    }
    let after = hitai_core::claude::tidy_rules(&key, &before).await?;
    hitai_core::set_rules(&after).map_err(|e| e.to_string())?;
    Ok(snapshot_of(&State::load()))
}

/// 규칙 파일을 사용자의 기본 편집기로 연다.
#[tauri::command]
fn open_rules_file() -> Result<String, String> {
    let path = hitai_core::rules_path().map_err(|e| e.to_string())?;
    if !path.exists() {
        hitai_core::set_rules(&[]).map_err(|e| e.to_string())?;
    }
    open::that(&path).map_err(|e| format!("파일을 열지 못했습니다: {e}"))?;
    Ok(path.to_string_lossy().to_string())
}

#[tauri::command]
fn delete_rule(rule: String) -> Result<Snapshot, String> {
    hitai_core::remove_rule(&rule).map_err(|e| e.to_string())?;
    Ok(snapshot_of(&State::load()))
}

#[tauri::command]
fn reboot() -> Result<Snapshot, String> {
    let mut state = State::load();
    state.reboot();
    state.save().map_err(|e| e.to_string())?;
    Ok(snapshot_of(&state))
}

#[tauri::command]
fn save_api_key(key: String) -> Result<Snapshot, String> {
    Config {
        api_key: key.trim().to_string(),
    }
    .save()
    .map_err(|e| e.to_string())?;
    Ok(snapshot_of(&State::load()))
}

/// 도구 연동을 켜고 끈다.
#[tauri::command]
fn set_tool_active(tool: String, active: bool) -> Result<String, String> {
    let tool = Tool::parse(&tool).ok_or_else(|| format!("모르는 도구입니다: {tool}"))?;
    if active {
        link::activate(tool)
    } else {
        link::deactivate(tool)
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        // Remembers window position, size and maximized state.
        .on_window_event(|w, event| window::on_event(w, event))
        .invoke_handler(tauri::generate_handler![
            get_snapshot,
            list_sessions,
            hit,
            delete_rule,
            reboot,
            save_api_key,
            set_tool_active,
            edit_rule,
            set_rules,
            tidy_rules,
            open_rules_file
        ])
        .run(tauri::generate_context!())
        .expect("HitAI를 실행하지 못했습니다");
}
