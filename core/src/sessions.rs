//! 최근 대화한 에이전트 세션 목록.
//!
//! 훅에 의존하지 않는다. 두 도구가 남기는 대화 기록 파일을 직접 훑는다.
//! 그래서 훅이 한 번도 돌지 않은 세션까지 잡히고, 도구 설정을 건드릴 필요도 없다.
//!
//! - Claude Code : `~/.claude/projects/<경로>/<세션UUID>.jsonl`
//! - Codex       : `~/.codex/sessions/**/rollout-<날짜>-<세션UUID>.jsonl`
//!
//! 두 형식 모두 줄 단위 JSON이고 어딘가에 `cwd`가 들어 있다.
//!
//! 정렬은 사용자가 마지막으로 말한 시각을 쓴다. 파일 수정 시각은 에이전트가 도구를
//! 쓰거나 답을 쓸 때마다 바뀌어서, 그것으로 정렬하면 목록 순서가 끊임없이 흔들린다.
//! 파일 수정 시각은 최근 12시간을 걸러내는 값싼 사전 필터로만 쓴다.

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 이보다 오래 조용한 세션은 목록에 넣지 않는다.
const STALE_AFTER_HOURS: i64 = 12;

/// 도구별로 내용을 읽어볼 최대 파일 수. 워크트리가 많은 환경에서 훑는 비용을 묶는다.
/// 도구마다 따로 세므로 한쪽이 많아도 다른 쪽이 목록에서 밀려나지 않는다.
const MAX_PER_TOOL: usize = 30;

/// `cwd`를 찾기 위해 읽어볼 줄 수.
const SCAN_LINES: usize = 60;

/// 마지막 발언을 찾기 위해 파일 끝에서 읽어볼 바이트 수.
/// 기록 파일은 수십 MB가 되므로 전체를 읽지 않는다. 도구 결과가 길어 끝부분에
/// 사용자 발언이 없으면 범위를 넓혀 다시 찾는다.
const TAIL_STEPS: [u64; 3] = [256 * 1024, 2 * 1024 * 1024, 12 * 1024 * 1024];

/// 목록에 보여줄 발언 길이.
const SNIPPET_CHARS: usize = 90;

#[derive(Debug, Clone, Serialize)]
pub struct Session {
    pub id: String,
    /// "claude" 또는 "codex".
    pub tool: String,
    pub tool_label: String,
    /// 세션이 돌고 있는 디렉터리.
    pub cwd: String,
    /// 목록에 보여줄 짧은 이름.
    pub label: String,
    /// 곁들일 설명. 브랜치 이름.
    pub detail: String,
    /// 이 세션에서 사용자가 마지막으로 한 말.
    pub last_message: String,
    /// 그 말을 한 시각. 목록 정렬 기준이다.
    pub last_spoke: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// "3분 전" 같은 표기.
    pub seen_ago: String,
}

fn tool_label(tool: &str) -> &'static str {
    match tool {
        "codex" => "Codex",
        _ => "Claude Code",
    }
}

fn ago(at: DateTime<Utc>) -> String {
    let secs = (Utc::now() - at).num_seconds().max(0);
    if secs < 60 {
        "방금".to_string()
    } else if secs < 3600 {
        format!("{}분 전", secs / 60)
    } else {
        format!("{}시간 전", secs / 3600)
    }
}

fn home() -> Option<PathBuf> {
    crate::base_home().ok()
}

fn mtime(path: &Path) -> Option<DateTime<Utc>> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(DateTime::<Utc>::from(modified))
}

/// 줄 단위 JSON 파일에서 특정 문자열 필드를 처음 나오는 값으로 뽑는다.
/// 기록 파일은 수십 MB가 될 수 있어 앞부분만 읽는다.
fn scan_field(path: &Path, keys: &[&str]) -> HashMap<String, String> {
    let mut found: HashMap<String, String> = HashMap::new();
    let Ok(file) = File::open(path) else {
        return found;
    };
    for line in BufReader::new(file).lines().take(SCAN_LINES) {
        let Ok(line) = line else { break };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        for key in keys {
            if found.contains_key(*key) {
                continue;
            }
            if let Some(v) = find_string(&value, key) {
                found.insert((*key).to_string(), v);
            }
        }
        if found.len() == keys.len() {
            break;
        }
    }
    found
}

/// JSON 안을 훑어 해당 키의 문자열 값을 찾는다.
fn find_string(value: &serde_json::Value, key: &str) -> Option<String> {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(serde_json::Value::String(s)) = map.get(key) {
                if !s.is_empty() {
                    return Some(s.clone());
                }
            }
            map.values().find_map(|v| find_string(v, key))
        }
        serde_json::Value::Array(items) => items.iter().find_map(|v| find_string(v, key)),
        _ => None,
    }
}

/// 파일 끝에서 `want` 바이트만 읽는다. 앞쪽에서 잘린 줄은 버린다.
fn tail(path: &Path, want: u64) -> Option<String> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(want);
    file.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf).to_string();
    if start == 0 {
        return Some(text);
    }
    // 중간부터 읽었으므로 첫 줄은 깨져 있다.
    text.find('\n').map(|i| text[i + 1..].to_string())
}

/// 목록에 올릴 한 줄로 다듬는다.
fn snippet(text: &str) -> Option<String> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let flat = flat.trim();
    if flat.is_empty() {
        return None;
    }
    // 사용자가 직접 한 말이 아닌 것들을 걸러낸다. 세션 기록에는 사용자 차례로
    // 들어가지만 사람이 타이핑한 것이 아닌 것들이 섞인다. 훅이 넣은 문장,
    // 도구나 하네스가 주입한 문맥, 붙여넣은 JSON이나 명령 출력.
    const INJECTED: [&str; 6] = [
        "[HitAI]",
        "Runtime context:",
        "Caveat:",
        "System:",
        "<",
        "=== ",
    ];
    if INJECTED.iter().any(|p| flat.starts_with(p)) {
        return None;
    }
    // 구조화된 데이터로 시작하면 사람이 쓴 문장이 아니다.
    if flat.starts_with('{') || flat.starts_with('[') || flat.starts_with("# ") {
        return None;
    }
    let mut out: String = flat.chars().take(SNIPPET_CHARS).collect();
    if flat.chars().count() > SNIPPET_CHARS {
        out.push('…');
    }
    Some(out)
}

/// 사용자가 마지막으로 한 말과 그 시각.
fn last_user_turn(path: &Path, tool: &str) -> Option<(String, Option<DateTime<Utc>>)> {
    let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
    let mut previous = 0u64;
    for want in TAIL_STEPS {
        if let Some(found) = search_tail(path, tool, want) {
            return Some(found);
        }
        // 이미 파일 전체를 읽었으면 더 넓혀도 소용없다.
        if want >= size || want == previous {
            break;
        }
        previous = want;
    }
    None
}

fn search_tail(path: &Path, tool: &str, want: u64) -> Option<(String, Option<DateTime<Utc>>)> {
    let text = tail(path, want)?;
    for line in text.lines().rev() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let found = match tool {
            "codex" => codex_user_text(&value),
            _ => claude_user_text(&value),
        };
        if let Some(found) = found.as_deref().and_then(snippet) {
            return Some((found, spoken_at(&value)));
        }
    }
    None
}

/// 그 줄에 적힌 시각. 두 도구 모두 최상위 `timestamp`에 ISO 문자열로 남긴다.
fn spoken_at(value: &serde_json::Value) -> Option<DateTime<Utc>> {
    let raw = value.get("timestamp")?.as_str()?;
    DateTime::parse_from_rfc3339(raw)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

/// Claude Code: `{type:"user", message:{content: "..." | [블록]}}`
fn claude_user_text(value: &serde_json::Value) -> Option<String> {
    if value.get("type").and_then(|v| v.as_str()) != Some("user") {
        return None;
    }
    let content = value.pointer("/message/content")?;
    match content {
        // 사람이 직접 입력한 말은 문자열로 들어온다.
        serde_json::Value::String(s) => Some(s.clone()),
        // 배열이면 도구 결과일 수 있다. 그건 사용자의 말이 아니다.
        serde_json::Value::Array(blocks) => {
            if blocks
                .iter()
                .any(|b| b.get("type").and_then(|v| v.as_str()) == Some("tool_result"))
            {
                return None;
            }
            blocks
                .iter()
                .find(|b| b.get("type").and_then(|v| v.as_str()) == Some("text"))
                .and_then(|b| b.get("text"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
        _ => None,
    }
}

/// Codex: `{payload:{role:"user", content:[{type:"input_text", text:"..."}]}}`
fn codex_user_text(value: &serde_json::Value) -> Option<String> {
    let payload = value.get("payload").unwrap_or(value);
    if payload.get("role").and_then(|v| v.as_str()) != Some("user") {
        return None;
    }
    payload
        .get("content")?
        .as_array()?
        .iter()
        .find(|b| b.get("type").and_then(|v| v.as_str()) == Some("input_text"))
        .and_then(|b| b.get("text"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// 경로 마지막 조각. 목록에서 세션을 알아보는 가장 빠른 단서다.
fn basename(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "(경로 없음)".to_string())
}

/* ---------- Claude Code ---------- */

fn claude_candidates(cutoff: DateTime<Utc>) -> Vec<(PathBuf, DateTime<Utc>)> {
    let Some(root) = home().map(|h| h.join(".claude").join("projects")) else {
        return Vec::new();
    };
    let Ok(projects) = fs::read_dir(&root) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    for project in projects.filter_map(|e| e.ok()) {
        let Ok(files) = fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.filter_map(|e| e.ok()) {
            let path = file.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(at) = mtime(&path) {
                if at >= cutoff {
                    out.push((path, at));
                }
            }
        }
    }
    out
}

fn claude_session(path: &Path, at: DateTime<Utc>) -> Option<Session> {
    let id = path.file_stem()?.to_string_lossy().to_string();
    let fields = scan_field(path, &["cwd", "gitBranch"]);
    let cwd = fields.get("cwd").cloned().unwrap_or_default();
    let (message, spoke) = last_user_turn(path, "claude").unwrap_or_default();
    // 사용자 발언 시각을 못 찾으면 파일 수정 시각으로 대신한다.
    let spoke = spoke.unwrap_or(at);
    Some(Session {
        id,
        tool: "claude".into(),
        tool_label: tool_label("claude").into(),
        label: basename(&cwd),
        detail: fields.get("gitBranch").cloned().unwrap_or_default(),
        last_message: message,
        last_spoke: spoke,
        cwd,
        last_seen: at,
        seen_ago: ago(spoke),
    })
}

/* ---------- Codex ---------- */

fn collect_rollouts(dir: &Path, cutoff: DateTime<Utc>, out: &mut Vec<(PathBuf, DateTime<Utc>)>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_dir() {
            collect_rollouts(&path, cutoff, out);
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("rollout-") || !name.ends_with(".jsonl") {
            continue;
        }
        if let Some(at) = mtime(&path) {
            if at >= cutoff {
                out.push((path, at));
            }
        }
    }
}

fn codex_session(path: &Path, at: DateTime<Utc>) -> Option<Session> {
    let fields = scan_field(path, &["id", "cwd"]);
    // 파일 이름 끝의 UUID가 세션 식별자다. 내용에서 찾지 못하면 이걸 쓴다.
    let id = fields.get("id").cloned().or_else(|| {
        let stem = path.file_stem()?.to_string_lossy().to_string();
        stem.rsplitn(6, '-')
            .collect::<Vec<_>>()
            .into_iter()
            .take(5)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("-")
            .into()
    })?;
    let cwd = fields.get("cwd").cloned().unwrap_or_default();
    let (message, spoke) = last_user_turn(path, "codex").unwrap_or_default();
    let spoke = spoke.unwrap_or(at);
    Some(Session {
        label: basename(&cwd),
        // Codex의 자동 생성 대화 제목은 내용과 동떨어질 때가 많아 쓰지 않는다.
        detail: String::new(),
        last_message: message,
        last_spoke: spoke,
        id,
        tool: "codex".into(),
        tool_label: tool_label("codex").into(),
        cwd,
        last_seen: at,
        seen_ago: ago(spoke),
    })
}

/* ---------- 캐시 ---------- */

/// 파일 하나에서 뽑아낸 정보를 수정 시각과 함께 담아 둔다.
/// 목록을 2초마다 새로 고쳐도 바뀐 파일만 다시 읽으면 되게 한다.
type Cache = HashMap<PathBuf, (DateTime<Utc>, Session)>;

fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 캐시에 있고 수정 시각이 그대로면 그것을 쓴다. 아니면 읽어서 캐시에 넣는다.
fn cached(path: &Path, at: DateTime<Utc>, build: impl FnOnce() -> Option<Session>) -> Option<Session> {
    if let Ok(map) = cache().lock() {
        if let Some((seen, session)) = map.get(path) {
            if *seen == at {
                let mut session = session.clone();
                // 경과 시간 표기는 매번 다시 계산한다.
                session.seen_ago = ago(session.last_spoke);
                return Some(session);
            }
        }
    }
    let session = build()?;
    if let Ok(mut map) = cache().lock() {
        map.insert(path.to_path_buf(), (at, session.clone()));
        // 오래된 항목이 무한히 쌓이지 않게 한다.
        if map.len() > 400 {
            let cutoff = Utc::now() - Duration::hours(STALE_AFTER_HOURS);
            map.retain(|_, (seen, _)| *seen >= cutoff);
        }
    }
    Some(session)
}

/* ---------- 목록 ---------- */

/// 최근 활동 순으로 세션을 돌려준다.
pub fn list() -> Vec<Session> {
    let cutoff = Utc::now() - Duration::hours(STALE_AFTER_HOURS);

    let mut claude = claude_candidates(cutoff);
    let mut codex: Vec<(PathBuf, DateTime<Utc>)> = Vec::new();
    if let Some(dir) = home().map(|h| h.join(".codex").join("sessions")) {
        collect_rollouts(&dir, cutoff, &mut codex);
    }

    // 최근 것부터 훑고, 내용을 읽는 파일 수를 제한한다.
    claude.sort_by(|a, b| b.1.cmp(&a.1));
    codex.sort_by(|a, b| b.1.cmp(&a.1));
    claude.truncate(MAX_PER_TOOL);
    codex.truncate(MAX_PER_TOOL);

    let mut out: Vec<Session> = Vec::new();
    for (path, at) in claude {
        if let Some(s) = cached(&path, at, || claude_session(&path, at)) {
            out.push(s);
        }
    }
    for (path, at) in codex {
        if let Some(s) = cached(&path, at, || codex_session(&path, at)) {
            out.push(s);
        }
    }

    // 사용자가 말한 시각 기준 최신 순.
    //
    // 사용자가 한 번도 말하지 않은 세션은 뒤로 보낸다. 자동화가 띄운 세션은
    // 사용자 차례가 주입된 문맥뿐이어서 정렬 기준이 없고, 파일 수정 시각으로
    // 대신하면 실제 대화보다 위로 올라와 순서가 계속 흔들린다.
    out.sort_by(|a, b| {
        let spoken = |s: &Session| !s.last_message.is_empty();
        spoken(b)
            .cmp(&spoken(a))
            .then(b.last_spoke.cmp(&a.last_spoke))
    });
    out
}

/// 가장 최근에 대화한 세션.
pub fn latest() -> Option<Session> {
    list().into_iter().next()
}

pub fn find(id: &str) -> Option<Session> {
    list().into_iter().find(|s| s.id == id)
}

/// 이름 조각으로 세션을 찾는다. 식별자를 외우지 않아도 되게 한다.
pub fn resolve(name: &str) -> Option<Session> {
    let needle = name.trim().to_lowercase();
    if needle.is_empty() {
        return None;
    }
    let all = list();
    all.iter()
        .find(|s| s.id.to_lowercase() == needle)
        .or_else(|| all.iter().find(|s| s.id.to_lowercase().starts_with(&needle)))
        .or_else(|| all.iter().find(|s| s.label.to_lowercase().contains(&needle)))
        .or_else(|| all.iter().find(|s| s.detail.to_lowercase().contains(&needle)))
        .or_else(|| all.iter().find(|s| s.last_message.to_lowercase().contains(&needle)))
        .or_else(|| all.iter().find(|s| s.cwd.to_lowercase().contains(&needle)))
        .or_else(|| all.iter().find(|s| s.tool == needle))
        .cloned()
}
