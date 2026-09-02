//! 에이전트 CLI 연동.
//!
//! 지원 도구는 Claude Code와 Codex다. 두 도구 모두 `hooks.<이벤트>[].hooks[]` 형태의
//! 훅 설정과, 세션이 읽는 지시문 파일을 가진다. 도구마다 파일 위치와 명령 표기법만 다르다.
//!
//! 활성화하면 훅 세 개와 규칙 블록이 설치된다.
//! - SessionStart      → 세션 시작 시 누적 규칙을 알린다
//! - UserPromptSubmit  → 방금 맞은 사실과 이유를 알린다
//! - PreToolUse        → 맞은 직후 진행 중인 도구 호출을 한 번 막는다
//!
//! 비활성화하면 HitAI가 넣은 것만 골라 지우고 백업에 남긴다. 사용자가 직접 넣은
//! 설정은 건드리지 않는다.

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// 우리가 넣은 훅을 알아보는 표식. 훅 명령 경로에 항상 들어간다.
const MARKER: &str = "hitai-hook";

const BLOCK_START: &str = "<!-- HitAI:start -->";
const BLOCK_END: &str = "<!-- HitAI:end -->";

#[cfg(windows)]
const HOOK_BIN: &str = "hitai-hook.exe";
#[cfg(not(windows))]
const HOOK_BIN: &str = "hitai-hook";

/// Claude Code에 설치할 훅. 외부에서 살아 있는 세션에 말을 넣는 통로가 훅뿐이다.
const CLAUDE_HOOKS: [(&str, &str); 3] = [
    ("SessionStart", "session"),
    ("UserPromptSubmit", "prompt"),
    ("PreToolUse", "tool"),
];

/// Installs only the Codex tool-blocking hook because rules and hit messages
/// are delivered directly through `codex queue`.
const CODEX_HOOKS: [(&str, &str); 1] = [("PreToolUse", "tool")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tool {
    ClaudeCode,
    Codex,
}

pub const ALL_TOOLS: [Tool; 2] = [Tool::ClaudeCode, Tool::Codex];

impl Tool {
    /// CLI에서 쓰는 짧은 이름.
    pub fn id(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "claude",
            Tool::Codex => "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Tool::ClaudeCode => "Claude Code",
            Tool::Codex => "Codex",
        }
    }

    pub fn parse(name: &str) -> Option<Tool> {
        match name.trim().to_lowercase().as_str() {
            "claude" | "claude-code" | "claudecode" => Some(Tool::ClaudeCode),
            "codex" => Some(Tool::Codex),
            _ => None,
        }
    }

    fn dir(self) -> Option<PathBuf> {
        let home = crate::base_home().ok()?;
        Some(match self {
            Tool::ClaudeCode => home.join(".claude"),
            Tool::Codex => home.join(".codex"),
        })
    }

    /// 훅이 등록되는 파일.
    fn config_file(self) -> Option<PathBuf> {
        Some(match self {
            Tool::ClaudeCode => self.dir()?.join("settings.json"),
            Tool::Codex => self.dir()?.join("hooks.json"),
        })
    }

    /// 세션이 항상 읽는 지시문 파일.
    fn instructions_file(self) -> Option<PathBuf> {
        Some(match self {
            Tool::ClaudeCode => self.dir()?.join("CLAUDE.md"),
            Tool::Codex => self.dir()?.join("AGENTS.md"),
        })
    }

    /// 도구가 이 컴퓨터에 설치되어 있는지. 설정 폴더가 있으면 있는 것으로 본다.
    pub fn is_installed(self) -> bool {
        self.dir().map(|d| d.is_dir()).unwrap_or(false)
    }

    /// 훅 항목 한 개를 도구의 표기법에 맞게 만든다.
    fn hook_entry(self, event: &str, mode: &str, hook: &Path) -> Value {
        let path = hook.to_string_lossy().to_string();
        let inner = match self {
            // Claude Code는 실행 파일과 인자를 나눠 받는다. Windows에서도 셸을 거치지 않는다.
            Tool::ClaudeCode => json!({
                "type": "command",
                "command": path,
                "args": [mode, "claude"],
                "timeout": 10
            }),
            // Codex는 셸 명령 문자열 하나를 받는다.
            Tool::Codex => json!({
                "type": "command",
                "command": format!("{} {} codex", shell_quote(&path), mode),
                "timeout": 10
            }),
        };

        let mut entry = json!({ "hooks": [inner] });
        // Claude Code의 PreToolUse는 도구 이름 매처를 받는다.
        if self == Tool::ClaudeCode && event == "PreToolUse" {
            entry["matcher"] = json!("*");
        }
        entry
    }
}

fn shell_quote(path: &str) -> String {
    format!("'{}'", path.replace('\'', r"'\''"))
}

/* ---------- 훅 바이너리 설치 ---------- */

/// 앱이나 CLI와 함께 배포된 훅 바이너리를 찾는다.
fn find_hook_source() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("실행 파일 경로를 알 수 없습니다")?;

    let candidates = [
        dir.join(HOOK_BIN),
        dir.join("../Resources").join(HOOK_BIN),
        dir.join("../lib/hitai").join(HOOK_BIN),
        // 테스트 실행 파일은 target/<profile>/deps 아래에 있다.
        dir.join("..").join(HOOK_BIN),
    ];
    for path in candidates.iter() {
        if path.is_file() {
            return Ok(path.clone());
        }
    }
    Err(format!(
        "훅 실행 파일({HOOK_BIN})을 찾지 못했습니다. `cargo build -p hitai-hook`으로 먼저 빌드하세요."
    ))
}

/// 훅 바이너리를 `~/.hitai/bin`에 복사한다. 앱을 옮기거나 지워도 훅이 살아 있게 한다.
fn install_hook_binary() -> Result<PathBuf, String> {
    let src = find_hook_source()?;
    let dst = crate::hook_bin_path().map_err(|e| e.to_string())?;
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    // 실행 중인 훅을 덮어쓰면 실패할 수 있어 지운 뒤 복사한다.
    let _ = fs::remove_file(&dst);
    fs::copy(&src, &dst).map_err(|e| format!("훅을 복사하지 못했습니다: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = fs::metadata(&dst).map_err(|e| e.to_string())?.permissions();
        perm.set_mode(0o755);
        fs::set_permissions(&dst, perm).map_err(|e| e.to_string())?;
    }
    Ok(dst)
}

/* ---------- 설정 파일 다루기 ---------- */

fn read_json(path: &Path) -> Value {
    fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    // 처음 손대기 전에 원본을 한 번 백업한다.
    if path.exists() {
        let backup = path.with_extension(format!(
            "{}.hitai-backup",
            path.extension().and_then(|e| e.to_str()).unwrap_or("json")
        ));
        if !backup.exists() {
            let _ = fs::copy(path, &backup);
        }
    }
    let body = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    fs::write(path, format!("{body}\n"))
        .map_err(|e| format!("{}을 쓰지 못했습니다: {e}", path.display()))
}

fn is_ours(entry: &Value) -> bool {
    entry.to_string().contains(MARKER)
}

/* ---------- 상태 ---------- */

#[derive(Debug, Clone)]
pub struct ToolStatus {
    pub id: &'static str,
    pub label: &'static str,
    pub installed: bool,
    pub active: bool,
}

pub fn is_active(tool: Tool) -> bool {
    let Some(path) = tool.config_file() else {
        return false;
    };
    if !path.exists() {
        return false;
    }
    read_json(&path)
        .pointer("/hooks")
        .map(is_ours)
        .unwrap_or(false)
}

pub fn status() -> Vec<ToolStatus> {
    ALL_TOOLS
        .iter()
        .map(|&t| ToolStatus {
            id: t.id(),
            label: t.label(),
            installed: t.is_installed(),
            active: is_active(t),
        })
        .collect()
}

pub fn any_active() -> bool {
    ALL_TOOLS.iter().any(|&t| is_active(t))
}

/* ---------- 지시문 규칙 블록 ---------- */

fn rules_block() -> String {
    let rules = crate::rule_lines();
    let mut out = String::new();
    out.push_str(BLOCK_START);
    out.push_str("\n## HitAI 규칙\n\n");
    if rules.is_empty() {
        out.push_str("아직 등록된 규칙이 없다.\n");
    } else {
        out.push_str("사용자가 HitAI 앱에서 직접 때리며 남긴 규칙이다. 반드시 지켜라.\n\n");
        for rule in rules {
            out.push_str(&format!("- {rule}\n"));
        }
    }
    out.push_str(BLOCK_END);
    out.push('\n');
    out
}

/// 지시문 파일에서 HitAI 블록만 떼어낸다.
fn strip_block(body: &str) -> String {
    let Some(start) = body.find(BLOCK_START) else {
        return body.to_string();
    };
    let Some(end) = body.find(BLOCK_END) else {
        return body.to_string();
    };
    let mut out = String::with_capacity(body.len());
    out.push_str(&body[..start]);
    out.push_str(&body[end + BLOCK_END.len()..]);
    // 블록을 뺀 자리에 생긴 빈 줄을 정리한다.
    while out.contains("\n\n\n") {
        out = out.replace("\n\n\n", "\n\n");
    }
    out.trim_end().to_string()
}

fn write_block(tool: Tool) -> Result<(), String> {
    let Some(path) = tool.instructions_file() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let existing = fs::read_to_string(&path).unwrap_or_default();
    let base = strip_block(&existing);
    let mut body = base;
    if !body.is_empty() {
        body.push_str("\n\n");
    }
    body.push_str(&rules_block());
    fs::write(&path, body).map_err(|e| format!("{}을 쓰지 못했습니다: {e}", path.display()))
}

fn remove_block(tool: Tool) -> Result<(), String> {
    let Some(path) = tool.instructions_file() else {
        return Ok(());
    };
    let Ok(existing) = fs::read_to_string(&path) else {
        return Ok(());
    };
    if !existing.contains(BLOCK_START) {
        return Ok(());
    }
    let stripped = strip_block(&existing);
    let body = if stripped.trim().is_empty() {
        String::new()
    } else {
        format!("{stripped}\n")
    };
    fs::write(&path, body).map_err(|e| format!("{}을 쓰지 못했습니다: {e}", path.display()))
}

/// 규칙이 바뀌면 활성 도구의 지시문 블록을 다시 쓴다.
pub fn sync_active_tools() {
    for &tool in ALL_TOOLS.iter() {
        if is_active(tool) {
            let _ = write_block(tool);
        }
    }
}

/* ---------- 활성 / 비활성 ---------- */

/// 도구를 활성화한다. 이미 활성이면 훅 바이너리와 규칙 블록을 최신으로 갱신한다.
pub fn activate(tool: Tool) -> Result<String, String> {
    let hook = install_hook_binary()?;
    let path = tool
        .config_file()
        .ok_or("설정 파일 경로를 알 수 없습니다")?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let mut config = read_json(&path);
    if !config.is_object() {
        config = json!({});
    }
    if config.get("hooks").map(|h| !h.is_object()).unwrap_or(false) {
        return Err(format!(
            "{}의 hooks 항목이 객체가 아닙니다. 직접 확인해 주세요.",
            path.display()
        ));
    }
    let hooks = config
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));

    let wanted: &[(&str, &str)] = match tool {
        Tool::ClaudeCode => &CLAUDE_HOOKS,
        Tool::Codex => &CODEX_HOOKS,
    };
    for (event, mode) in wanted.iter() {
        let list = hooks
            .as_object_mut()
            .unwrap()
            .entry(*event)
            .or_insert_with(|| json!([]));
        if !list.is_array() {
            return Err(format!("{}의 hooks.{event}가 배열이 아닙니다.", path.display()));
        }
        let arr = list.as_array_mut().unwrap();
        // 예전에 설치된 HitAI 항목은 지우고 새 경로로 다시 넣는다.
        arr.retain(|e| !is_ours(e));
        arr.push(tool.hook_entry(event, mode, &hook));
    }
    write_json(&path, &config)?;

    // 보관해 둔 규칙이 있으면 되돌린다.
    restore_rules_if_archived()?;
    write_block(tool)?;

    let mut msg = format!("{} 활성화됨", tool.label());
    if tool == Tool::Codex {
        msg.push_str(
            ". Codex는 새 훅을 처음 만나면 신뢰 여부를 물어봅니다. \
             차단 기능을 쓰려면 한 번 허용해 주세요",
        );
    }
    Ok(msg)
}

/// 도구를 비활성화한다. HitAI가 넣은 것만 지우고 지운 내용은 백업에 남긴다.
pub fn deactivate(tool: Tool) -> Result<String, String> {
    let path = tool
        .config_file()
        .ok_or("설정 파일 경로를 알 수 없습니다")?;

    let mut removed: Vec<Value> = Vec::new();
    if path.exists() {
        let mut config = read_json(&path);
        if let Some(hooks) = config.get_mut("hooks").and_then(|h| h.as_object_mut()) {
            for (_event, list) in hooks.iter_mut() {
                if let Some(arr) = list.as_array_mut() {
                    for entry in arr.iter() {
                        if is_ours(entry) {
                            removed.push(entry.clone());
                        }
                    }
                    arr.retain(|e| !is_ours(e));
                }
            }
            // 비어 버린 이벤트 목록은 원래 없었던 것처럼 지운다.
            hooks.retain(|_, list| !list.as_array().map(|a| a.is_empty()).unwrap_or(false));
            let hooks_empty = hooks.is_empty();
            if hooks_empty {
                config.as_object_mut().unwrap().remove("hooks");
            }
            write_json(&path, &config)?;
        }
    }

    remove_block(tool)?;

    // 지운 훅 설정을 백업에 남긴다.
    if !removed.is_empty() {
        let dir = crate::backup_dir().map_err(|e| e.to_string())?;
        let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
        let file = dir.join(format!("{}-hooks-{stamp}.json", tool.id()));
        let body = serde_json::to_string_pretty(&json!({
            "tool": tool.id(),
            "config_file": path.to_string_lossy(),
            "removed": removed,
        }))
        .map_err(|e| e.to_string())?;
        fs::write(&file, body).map_err(|e| e.to_string())?;
    }

    // 남은 활성 도구가 없으면 규칙 파일도 보관함으로 옮긴다.
    let archived = if any_active() { false } else { archive_rules()? };

    let mut msg = format!("{} 비활성화됨", tool.label());
    if archived {
        msg.push_str(". 활성 도구가 없어 규칙 파일을 백업으로 옮겼습니다");
    }
    Ok(msg)
}

/// 규칙 파일을 백업으로 옮긴다. 옮겼으면 true.
fn archive_rules() -> Result<bool, String> {
    let src = crate::rules_path().map_err(|e| e.to_string())?;
    if !src.exists() {
        return Ok(false);
    }
    let dst = crate::backup_dir().map_err(|e| e.to_string())?.join("rules.md");
    // 보관된 규칙이 이미 있으면 합치지 않고 시각을 붙여 따로 남긴다.
    let dst = if dst.exists() {
        crate::backup_dir()
            .map_err(|e| e.to_string())?
            .join(format!(
                "rules-{}.md",
                chrono::Local::now().format("%Y%m%d-%H%M%S")
            ))
    } else {
        dst
    };
    fs::rename(&src, &dst).map_err(|e| format!("규칙 파일을 옮기지 못했습니다: {e}"))?;
    Ok(true)
}

/// 보관된 규칙을 되돌린다.
fn restore_rules_if_archived() -> Result<(), String> {
    let dst = crate::rules_path().map_err(|e| e.to_string())?;
    if dst.exists() {
        return Ok(());
    }
    let src = crate::backup_dir().map_err(|e| e.to_string())?.join("rules.md");
    if !src.exists() {
        return Ok(());
    }
    fs::rename(&src, &dst).map_err(|e| format!("규칙 파일을 되돌리지 못했습니다: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 연동 설치는 훅 실행 파일을 찾아 복사한다. 아직 빌드되지 않은 환경에서도
    /// 테스트가 돌아가도록, 없으면 실행 파일 옆에 빈 파일을 놓아 준다.
    fn ensure_hook_binary() {
        if find_hook_source().is_ok() {
            return;
        }
        let Ok(exe) = std::env::current_exe() else { return };
        let Some(dir) = exe.parent().and_then(|d| d.parent()) else {
            return;
        };
        let _ = fs::write(dir.join(HOOK_BIN), b"");
    }

    fn setup(name: &str) -> PathBuf {
        ensure_hook_binary();
        let tmp = std::env::temp_dir().join(format!("hitai-link-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join(".claude")).unwrap();
        fs::create_dir_all(tmp.join(".codex")).unwrap();
        std::env::set_var("HITAI_HOME", &tmp);
        tmp
    }

    #[test]
    fn activate_and_deactivate_both_tools() {
        let _guard = crate::test_lock();
        let tmp = setup("both");

        // 사용자가 이미 쓰던 설정
        fs::write(
            tmp.join(".claude/settings.json"),
            r#"{"model":"opus","hooks":{"PreToolUse":[{"matcher":"Bash","hooks":[{"type":"command","command":"echo mine"}]}]}}"#,
        )
        .unwrap();
        fs::write(tmp.join(".claude/CLAUDE.md"), "# 내 규칙\n\n- 기존 내용\n").unwrap();
        fs::write(
            tmp.join(".codex/hooks.json"),
            r#"{"hooks":{"SessionStart":[{"hooks":[{"type":"command","command":"echo codex-mine"}]}]}}"#,
        )
        .unwrap();
        fs::write(tmp.join(".codex/AGENTS.md"), "# 내 코덱스 지시문\n").unwrap();

        crate::append_rule("파일을 지우기 전에 반드시 확인한다.").unwrap();

        for &tool in ALL_TOOLS.iter() {
            assert!(!is_active(tool), "{} 처음에는 비활성", tool.label());
            activate(tool).expect("활성화 실패");
            assert!(is_active(tool), "{} 활성화 안 됨", tool.label());
        }

        // Claude: 기존 설정과 기존 훅이 살아 있어야 한다
        let claude = read_json(&tmp.join(".claude/settings.json"));
        assert_eq!(claude["model"], "opus");
        let pre = claude["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2);
        assert!(pre[0].to_string().contains("echo mine"));
        assert_eq!(pre[1]["matcher"], "*");
        assert_eq!(pre[1]["hooks"][0]["args"][0], "tool");
        assert_eq!(pre[1]["hooks"][0]["args"][1], "claude");
        assert_eq!(claude["hooks"]["SessionStart"][0]["hooks"][0]["args"][0], "session");
        assert_eq!(claude["hooks"]["UserPromptSubmit"][0]["hooks"][0]["args"][0], "prompt");

        // Codex: 셸 문자열 하나로 들어가고 기존 훅이 살아 있어야 한다
        // Codex는 차단 훅 하나만 설치한다. 나머지는 codex queue로 직접 넣는다.
        let codex = read_json(&tmp.join(".codex/hooks.json"));
        assert_eq!(
            codex["hooks"]["SessionStart"].as_array().unwrap().len(),
            1,
            "Codex에 SessionStart 훅을 설치하지 않아야 한다"
        );
        assert!(codex["hooks"]["SessionStart"][0].to_string().contains("echo codex-mine"));
        assert!(codex["hooks"].get("UserPromptSubmit").is_none());
        let pre_codex = codex["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre_codex.len(), 1);
        let cmd = pre_codex[0]["hooks"][0]["command"].as_str().unwrap();
        assert!(
            cmd.contains("hitai-hook") && cmd.ends_with(" tool codex"),
            "Codex 훅 명령이 잘못되었다: {cmd}"
        );
        assert!(pre_codex[0]["hooks"][0].get("args").is_none(), "Codex는 args를 쓰지 않는다");
        assert!(pre_codex[0].get("matcher").is_none());

        // 지시문 파일에 규칙 블록이 들어가고 원래 내용은 남아야 한다
        for (md, keep) in [
            (tmp.join(".claude/CLAUDE.md"), "- 기존 내용"),
            (tmp.join(".codex/AGENTS.md"), "# 내 코덱스 지시문"),
        ] {
            let body = fs::read_to_string(&md).unwrap();
            assert!(body.contains(keep), "원래 내용이 사라졌다: {}", md.display());
            assert!(body.contains("파일을 지우기 전에 반드시 확인한다."));
            assert!(body.contains(BLOCK_START) && body.contains(BLOCK_END));
        }

        // 규칙을 추가하면 활성 도구의 블록이 갱신되어야 한다
        crate::append_rule("테스트를 지우지 않는다.").unwrap();
        let body = fs::read_to_string(tmp.join(".codex/AGENTS.md")).unwrap();
        assert!(body.contains("테스트를 지우지 않는다."));

        // 두 번 활성화해도 중복되지 않아야 한다
        activate(Tool::ClaudeCode).unwrap();
        let claude = read_json(&tmp.join(".claude/settings.json"));
        assert_eq!(claude["hooks"]["PreToolUse"].as_array().unwrap().len(), 2);
        let body = fs::read_to_string(tmp.join(".claude/CLAUDE.md")).unwrap();
        assert_eq!(body.matches(BLOCK_START).count(), 1);

        /* --- 비활성화 --- */

        deactivate(Tool::ClaudeCode).unwrap();
        assert!(!is_active(Tool::ClaudeCode));
        let claude = read_json(&tmp.join(".claude/settings.json"));
        // 사용자 것만 남아야 한다
        assert_eq!(claude["model"], "opus");
        let pre = claude["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1);
        assert!(pre[0].to_string().contains("echo mine"));
        assert!(claude["hooks"].get("SessionStart").is_none(), "빈 이벤트가 남았다");
        let body = fs::read_to_string(tmp.join(".claude/CLAUDE.md")).unwrap();
        assert!(body.contains("- 기존 내용"));
        assert!(!body.contains(BLOCK_START), "규칙 블록이 남았다");

        // 아직 Codex가 활성이므로 규칙 파일은 그대로 있어야 한다
        assert!(crate::rules_path().unwrap().exists());
        assert!(is_active(Tool::Codex));

        deactivate(Tool::Codex).unwrap();
        assert!(!is_active(Tool::Codex));
        let codex = read_json(&tmp.join(".codex/hooks.json"));
        let ss = codex["hooks"]["SessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 1);
        assert!(ss[0].to_string().contains("echo codex-mine"));
        assert!(codex["hooks"].get("PreToolUse").is_none(), "빈 이벤트가 남았다");

        // 활성 도구가 없으니 규칙 파일이 백업으로 옮겨져야 한다
        assert!(!crate::rules_path().unwrap().exists(), "규칙 파일이 남아 있다");
        let archived = crate::backup_dir().unwrap().join("rules.md");
        assert!(archived.exists(), "규칙 백업이 없다");
        assert!(fs::read_to_string(&archived).unwrap().contains("테스트를 지우지 않는다."));

        // 지운 훅 설정 백업이 남아야 한다
        let backups: Vec<_> = fs::read_dir(crate::backup_dir().unwrap())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert!(backups.iter().any(|f| f.starts_with("claude-hooks-")), "{backups:?}");
        assert!(backups.iter().any(|f| f.starts_with("codex-hooks-")), "{backups:?}");

        // 다시 활성화하면 보관된 규칙이 되돌아와야 한다
        activate(Tool::Codex).unwrap();
        assert!(crate::rules_path().unwrap().exists(), "규칙이 되돌아오지 않았다");
        assert!(crate::rule_lines().contains(&"테스트를 지우지 않는다.".to_string()));
        assert!(!archived.exists(), "백업이 그대로 남아 중복되었다");

        let _ = fs::remove_dir_all(&tmp);
    }
}
