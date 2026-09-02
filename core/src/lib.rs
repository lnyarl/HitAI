//! HitAI 공용 코어.
//! 데스크톱 앱, CLI, 훅 바이너리가 모두 이 크레이트를 통해 같은 상태와 규칙을 다룬다.

pub mod claude;
pub mod deliver;
pub mod link;
pub mod sessions;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::PathBuf;

/// 때린 직후 "화가 난 상태"가 유지되는 시간. 이 안에 도구 호출이 오면 막는다.
pub const HOT_WINDOW_SECS: i64 = 180;

pub const MAX_HP: i32 = 100;

/// 한 대당 깎이는 내구도.
pub const HIT_DAMAGE: i32 = 7;

const RULES_HEADER: &str = "# HitAI 규칙\n\n\
사용자가 HitAI 앱에서 직접 때리며 남긴 규칙이다. 모든 세션에서 지켜야 한다.\n\n";

/// 기준이 되는 홈 경로.
///
/// `HITAI_HOME`이 설정되어 있으면 그것을 쓴다. Windows의 `dirs::home_dir()`은
/// 환경 변수가 아니라 시스템 API를 보기 때문에, 이 덮어쓰기가 없으면 테스트나
/// 이동식 설치에서 홈을 바꿀 수 없다.
pub fn base_home() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("HITAI_HOME") {
        let path = PathBuf::from(path);
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    dirs::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "홈 디렉터리를 찾을 수 없습니다"))
}

/// `~/.hitai`
pub fn home_dir() -> io::Result<PathBuf> {
    let dir = base_home()?.join(".hitai");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn state_path() -> io::Result<PathBuf> {
    Ok(home_dir()?.join("state.json"))
}

pub fn rules_path() -> io::Result<PathBuf> {
    Ok(home_dir()?.join("rules.md"))
}

pub fn config_path() -> io::Result<PathBuf> {
    Ok(home_dir()?.join("config.json"))
}

/// 비활성화한 도구의 설정 조각과 보관된 규칙이 들어가는 곳.
pub fn backup_dir() -> io::Result<PathBuf> {
    let dir = home_dir()?.join("backup");
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// 설치된 훅 바이너리 경로.
pub fn hook_bin_path() -> io::Result<PathBuf> {
    let name = if cfg!(windows) {
        "hitai-hook.exe"
    } else {
        "hitai-hook"
    };
    Ok(home_dir()?.join("bin").join(name))
}

/* ---------- 설정 ---------- */

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub api_key: String,
}

impl Config {
    pub fn load() -> Config {
        let Ok(path) = config_path() else {
            return Config::default();
        };
        let Ok(raw) = fs::read_to_string(path) else {
            return Config::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = config_path()?;
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(path, body)
    }
}

/* ---------- 상태 ---------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hit {
    pub id: String,
    pub at: DateTime<Utc>,
    /// 사용자가 적은 잘못. 비어 있을 수 있다.
    #[serde(default)]
    pub reason: String,
    /// 이 타격에서 뽑아낸 규칙 한 줄. 규칙을 만들지 않은 타격이면 None.
    #[serde(default)]
    pub rule: Option<String>,
    /// 로봇이 한 말.
    #[serde(default)]
    pub reply: String,
    /// 때릴 대상 세션. None이면 먼저 반응하는 세션이 받는다.
    #[serde(default)]
    pub target: Option<String>,
    /// 세션에 이미 주입되었는지.
    #[serde(default)]
    pub injected: bool,
    /// 도구 호출 차단에 이미 쓰였는지.
    #[serde(default)]
    pub blocked: bool,
}

impl Hit {
    /// 이 타격이 해당 세션에 전달될 것인지.
    pub fn targets(&self, session_id: &str) -> bool {
        match &self.target {
            None => true,
            Some(t) => t == session_id,
        }
    }

    /// 아직 도구 호출을 막을 수 있는 상태인지.
    pub fn is_hot(&self) -> bool {
        !self.blocked && Utc::now() < self.at + Duration::seconds(HOT_WINDOW_SECS)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct State {
    #[serde(default = "default_hp")]
    pub hp: i32,
    #[serde(default)]
    pub total_hits: u64,
    #[serde(default)]
    pub hits: Vec<Hit>,
}

fn default_hp() -> i32 {
    MAX_HP
}

impl Default for State {
    fn default() -> Self {
        State {
            hp: MAX_HP,
            total_hits: 0,
            hits: Vec::new(),
        }
    }
}

impl State {
    pub fn load() -> State {
        let Ok(path) = state_path() else {
            return State::default();
        };
        let Ok(raw) = fs::read_to_string(path) else {
            return State::default();
        };
        serde_json::from_str(&raw).unwrap_or_default()
    }

    pub fn save(&self) -> io::Result<()> {
        let path = state_path()?;
        let tmp = path.with_extension("json.tmp");
        let body = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        fs::write(&tmp, body)?;
        fs::rename(&tmp, &path)
    }

    /// 이 세션에 아직 전달되지 않은 타격들.
    pub fn pending_for(&self, session_id: &str) -> Vec<&Hit> {
        self.hits
            .iter()
            .filter(|h| !h.injected && h.targets(session_id))
            .collect()
    }

    /// 이 세션의 도구 호출을 막아야 하는 타격.
    pub fn hot_for(&self, session_id: &str) -> Option<&Hit> {
        self.hits
            .iter()
            .rev()
            .find(|h| h.is_hot() && h.targets(session_id))
    }

    /// 타격 하나를 기록한다.
    pub fn push_hit(&mut self, hit: Hit) {
        self.total_hits += 1;
        self.hp = (self.hp - HIT_DAMAGE).max(0);
        self.hits.push(hit);
        // 기록은 최근 200건만 유지한다.
        let len = self.hits.len();
        if len > 200 {
            self.hits.drain(0..len - 200);
        }
    }

    pub fn reboot(&mut self) {
        self.hp = MAX_HP;
    }
}

/* ---------- 규칙 ---------- */

/// 규칙 파일 전체를 읽는다. 없으면 헤더만 돌려준다.
pub fn read_rules() -> String {
    let Ok(path) = rules_path() else {
        return RULES_HEADER.to_string();
    };
    fs::read_to_string(path).unwrap_or_else(|_| RULES_HEADER.to_string())
}

/// 규칙 목록만 뽑는다.
pub fn rule_lines() -> Vec<String> {
    read_rules()
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- ").map(|s| s.to_string()))
        .collect()
}

/// 규칙 한 줄을 덧붙인다. 같은 규칙이 이미 있으면 넣지 않는다.
pub fn append_rule(rule: &str) -> io::Result<bool> {
    let rule = rule.trim();
    if rule.is_empty() {
        return Ok(false);
    }
    let path = rules_path()?;
    let mut body = if path.exists() {
        fs::read_to_string(&path)?
    } else {
        RULES_HEADER.to_string()
    };
    if body.lines().any(|l| l.trim() == format!("- {rule}")) {
        return Ok(false);
    }
    if !body.ends_with('\n') {
        body.push('\n');
    }
    body.push_str(&format!("- {rule}\n"));
    fs::write(&path, body)?;
    link::sync_active_tools();
    Ok(true)
}

/// 규칙 목록을 통째로 바꾼다. 사용자가 직접 고치거나 AI가 정리한 결과를 반영할 때 쓴다.
pub fn set_rules(rules: &[String]) -> io::Result<()> {
    let mut body = String::from(RULES_HEADER);
    let mut seen: Vec<String> = Vec::new();
    for rule in rules {
        let rule = rule.trim();
        if rule.is_empty() || seen.iter().any(|s| s == rule) {
            continue;
        }
        seen.push(rule.to_string());
        body.push_str(&format!("- {rule}\n"));
    }
    fs::write(rules_path()?, body)?;
    link::sync_active_tools();
    Ok(())
}

/// 규칙 한 줄을 다른 문장으로 바꾼다. 순서는 유지한다.
pub fn edit_rule(old: &str, new: &str) -> io::Result<()> {
    let new = new.trim();
    let old = old.trim();
    if new.is_empty() {
        return remove_rule(old);
    }
    let rules: Vec<String> = rule_lines()
        .into_iter()
        .map(|r| if r == old { new.to_string() } else { r })
        .collect();
    set_rules(&rules)
}

pub fn remove_rule(rule: &str) -> io::Result<()> {
    let path = rules_path()?;
    if !path.exists() {
        return Ok(());
    }
    let body = fs::read_to_string(&path)?;
    let target = format!("- {}", rule.trim());
    let kept: Vec<&str> = body.lines().filter(|l| l.trim() != target).collect();
    fs::write(&path, format!("{}\n", kept.join("\n")))?;
    link::sync_active_tools();
    Ok(())
}

/* ---------- 때리기 ---------- */

/// Describes how a hit message was delivered to a session.
#[derive(Debug, Clone, PartialEq)]
pub enum Delivery {
    /// 세션에 바로 넣었다.
    Sent,
    /// 훅이 다음 이벤트에 전달한다.
    WaitingForHook,
    /// 바로 넣으려 했지만 실패했다.
    Failed(String),
}

impl Delivery {
    pub fn describe(&self) -> String {
        match self {
            Delivery::Sent => "세션에 바로 전달했습니다".to_string(),
            Delivery::WaitingForHook => "다음 프롬프트나 도구 호출 때 전달됩니다".to_string(),
            Delivery::Failed(e) => format!("바로 전달하지 못했습니다: {e}"),
        }
    }
}

pub struct HitOutcome {
    pub reply: String,
    pub rule: Option<String>,
    pub state: State,
    pub delivery: Delivery,
}

/// 한 대 때린다. 이유를 적었으면 규칙으로 남는다.
///
/// API 키가 있으면 로봇의 대사와 규칙 문장을 모델이 만들고, 없거나 실패하면
/// 사용자가 적은 문장을 그대로 규칙으로 쓴다. 규칙이 쌓이는 것이 이 앱의 핵심이라
/// 모델 응답에 의존해서는 안 된다.
pub async fn hit(reason: &str, target: Option<String>) -> Result<HitOutcome, String> {
    let reason = reason.trim().to_string();
    let target = target.filter(|t| !t.trim().is_empty());
    let key = Config::load().api_key.trim().to_string();
    let hp_before = State::load().hp;

    let (reply, model_rule) = if key.is_empty() {
        (claude::offline_reply(&reason, hp_before), None)
    } else {
        match claude::discipline(&key, &reason, &rule_lines(), hp_before).await {
            Ok(v) => v,
            Err(e) => (
                format!("(응답 실패: {e}) ...알겠습니다. 고치겠습니다."),
                None,
            ),
        }
    };

    let rule = match (model_rule, reason.is_empty()) {
        (_, true) => None,
        (Some(r), false) => Some(r.trim().to_string()),
        (None, false) => Some(reason.clone()),
    };
    if let Some(r) = &rule {
        append_rule(r).map_err(|e| format!("규칙을 저장하지 못했습니다: {e}"))?;
    }

    let mut state = State::load();
    state.push_hit(Hit {
        id: format!("{}", Utc::now().timestamp_millis()),
        at: Utc::now(),
        reason: reason.clone(),
        rule: rule.clone(),
        reply: reply.clone(),
        target: target.clone(),
        injected: false,
        blocked: false,
    });

    // Codex 세션은 훅을 기다리지 않고 지금 바로 전달한다.
    let mut delivery = Delivery::WaitingForHook;
    if let Some(session) = target.as_deref().and_then(sessions::find) {
        if deliver::can_deliver_now(&session) {
            let message = deliver::message_for(&reason, &rule_lines(), state.total_hits);
            match deliver::deliver(&session, &message) {
                Ok(true) => {
                    delivery = Delivery::Sent;
                    // 이미 전달했으므로 훅이 같은 내용을 또 주입하지 않게 한다.
                    if let Some(last) = state.hits.last_mut() {
                        last.injected = true;
                    }
                }
                Ok(false) => {}
                Err(e) => delivery = Delivery::Failed(e),
            }
        }
    }

    state.save().map_err(|e| e.to_string())?;

    Ok(HitOutcome {
        reply,
        rule,
        state,
        delivery,
    })
}

/// 환경 변수를 건드리는 테스트끼리 겹치지 않게 한다.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// API 키 없이도 때린 기록과 규칙이 실제로 파일에 남아야 한다.
    #[test]
    fn hit_records_rule_without_api_key() {
        let _guard = test_lock();
        let tmp = std::env::temp_dir().join(format!("hitai-hit-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HITAI_HOME", &tmp);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // 이유를 적고 한 대
        let out = rt.block_on(hit("묻지도 않고 파일 지움", None)).unwrap();
        assert_eq!(out.rule.as_deref(), Some("묻지도 않고 파일 지움"));
        assert_eq!(out.state.total_hits, 1);
        assert_eq!(out.state.hp, MAX_HP - HIT_DAMAGE);
        assert!(!out.reply.is_empty(), "로봇이 아무 말도 하지 않았다");
        assert_eq!(rule_lines(), vec!["묻지도 않고 파일 지움"]);

        // 규칙 파일에 실제로 남았는지
        let body = fs::read_to_string(tmp.join(".hitai/rules.md")).unwrap();
        assert!(body.contains("- 묻지도 않고 파일 지움"));

        // 같은 이유로 또 때려도 규칙이 중복되지 않아야 한다
        rt.block_on(hit("묻지도 않고 파일 지움", None)).unwrap();
        assert_eq!(rule_lines().len(), 1);

        // 이유 없이 때리면 규칙은 생기지 않지만 기록과 손상은 남는다
        let out = rt.block_on(hit("  ", None)).unwrap();
        assert_eq!(out.rule, None);
        assert_eq!(out.state.total_hits, 3);
        assert_eq!(rule_lines().len(), 1);

        // 재부팅하면 내구도만 회복되고 규칙과 횟수는 유지된다
        let mut state = State::load();
        state.reboot();
        state.save().unwrap();
        let state = State::load();
        assert_eq!(state.hp, MAX_HP);
        assert_eq!(state.total_hits, 3);
        assert_eq!(rule_lines().len(), 1);

        let _ = fs::remove_dir_all(&tmp);
    }

    /// 대상을 지정한 타격은 그 세션에만 전달되어야 한다.
    #[test]
    fn hits_route_to_the_chosen_session() {
        let _guard = test_lock();
        let tmp = std::env::temp_dir().join(format!("hitai-route-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HITAI_HOME", &tmp);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();

        // A 세션을 겨냥해 한 대, 대상 없이 한 대
        rt.block_on(hit("A 세션이 잘못함", Some("session-A".into()))).unwrap();
        rt.block_on(hit("누구든 잘못함", None)).unwrap();

        let state = State::load();

        // A는 자기 것과 대상 없는 것을 받는다
        assert_eq!(state.pending_for("session-A").len(), 2);
        // B는 대상 없는 것만 받는다
        let b = state.pending_for("session-B");
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].reason, "누구든 잘못함");

        // 차단도 대상을 따른다
        assert!(state.hot_for("session-A").is_some());
        assert!(state.hot_for("session-B").is_some());

        // 이미 차단에 쓴 타격은 다시 막지 않는다
        let mut state = State::load();
        for hit in state.hits.iter_mut() {
            hit.blocked = true;
        }
        state.save().unwrap();
        assert!(State::load().hot_for("session-A").is_none());

        // B가 전달받아도 A를 겨냥한 타격은 남아 있어야 한다
        let mut state = State::load();
        for hit in state.hits.iter_mut() {
            if hit.targets("session-B") {
                hit.injected = true;
            }
        }
        state.save().unwrap();
        let state = State::load();
        assert_eq!(state.pending_for("session-B").len(), 0);
        assert_eq!(
            state.pending_for("session-A").len(),
            1,
            "다른 세션이 전달받았다고 A의 타격이 사라졌다"
        );

        let _ = fs::remove_dir_all(&tmp);
    }

    /// 사용자가 규칙을 직접 고치고 지울 수 있어야 한다.
    #[test]
    fn rules_can_be_edited_by_hand() {
        let _guard = test_lock();
        let tmp = std::env::temp_dir().join(format!("hitai-rules-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(&tmp).unwrap();
        std::env::set_var("HITAI_HOME", &tmp);

        append_rule("파일 지우기 전에 확인").unwrap();
        append_rule("테스트 지우지 마").unwrap();
        assert_eq!(rule_lines().len(), 2);

        // 한 줄 고치기. 순서는 유지된다.
        edit_rule("파일 지우기 전에 확인", "파일을 지우기 전에 반드시 사용자에게 확인한다.").unwrap();
        assert_eq!(
            rule_lines(),
            vec![
                "파일을 지우기 전에 반드시 사용자에게 확인한다.",
                "테스트 지우지 마"
            ]
        );

        // 빈 문장으로 고치면 지워진다.
        edit_rule("테스트 지우지 마", "   ").unwrap();
        assert_eq!(rule_lines().len(), 1);

        // 목록 전체 교체. AI 정리 결과를 반영하는 경로다.
        set_rules(&[
            "파일을 지우기 전에 반드시 확인한다.".to_string(),
            "테스트를 지우거나 건너뛰지 않는다.".to_string(),
            // 중복과 빈 줄은 걸러진다
            "테스트를 지우거나 건너뛰지 않는다.".to_string(),
            "  ".to_string(),
        ])
        .unwrap();
        assert_eq!(rule_lines().len(), 2);
        assert!(fs::read_to_string(tmp.join(".hitai/rules.md"))
            .unwrap()
            .contains("- 테스트를 지우거나 건너뛰지 않는다."));

        let _ = fs::remove_dir_all(&tmp);
    }
}
