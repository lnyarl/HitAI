//! 살아 있는 세션에 훈육 내용을 직접 전달한다.
//!
//! Codex는 `codex queue`로 실행 중인 세션에 메시지를 넣을 수 있다. 훅을 거치지 않으므로
//! 다음 프롬프트를 기다리지 않고, 신뢰 절차도 필요 없다.
//!
//! Claude Code에는 외부에서 살아 있는 세션에 말을 넣는 통로가 없다. 그쪽은 훅이 맡는다.

use crate::sessions::Session;
use std::process::Command;

/// 세션에 지금 바로 전달할 수 있는지.
pub fn can_deliver_now(session: &Session) -> bool {
    session.tool == "codex" && codex_bin().is_some()
}

fn codex_bin() -> Option<String> {
    // PATH에서 찾고, 없으면 기본 설치 위치를 본다.
    if Command::new("codex").arg("--version").output().is_ok() {
        return Some("codex".to_string());
    }
    let home = dirs::home_dir()?;
    let path = home.join(".local").join("bin").join("codex");
    if path.is_file() {
        return Some(path.to_string_lossy().to_string());
    }
    None
}

/// 훈육 내용을 세션에 전달한다. 전달했으면 Ok(true).
pub fn deliver(session: &Session, message: &str) -> Result<bool, String> {
    if session.tool != "codex" {
        return Ok(false);
    }
    let bin = codex_bin().ok_or("codex 명령을 찾지 못했습니다")?;

    let output = Command::new(&bin)
        .arg("queue")
        .arg("--thread")
        .arg(&session.id)
        .arg("--message")
        .arg(message)
        .output()
        .map_err(|e| format!("codex queue 실행 실패: {e}"))?;

    if output.status.success() {
        return Ok(true);
    }
    let err = String::from_utf8_lossy(&output.stderr);
    let err = err.trim();
    // 종료된 세션에 넣으려 하면 실패한다. 이유를 그대로 올린다.
    Err(if err.is_empty() {
        "codex queue가 실패했습니다".to_string()
    } else {
        err.lines().last().unwrap_or(err).to_string()
    })
}

/// 세션에 넣을 훈육 메시지.
pub fn message_for(reason: &str, rules: &[String], total_hits: u64) -> String {
    let mut out = String::from("[HitAI] 사용자가 방금 HitAI 앱에서 너를 때렸다.");
    if reason.trim().is_empty() {
        out.push_str(" 이유는 적지 않았다.");
    } else {
        out.push_str(&format!(" 맞은 이유: {}", reason.trim()));
    }
    out.push_str(&format!(" 지금까지 {total_hits}대 맞았다.\n"));

    if !rules.is_empty() {
        out.push_str("\n지금까지 누적된 훈육 규칙:\n");
        for rule in rules {
            out.push_str(&format!("- {rule}\n"));
        }
    }
    out.push_str(
        "\n하던 일을 멈추고, 무엇을 잘못했는지 한 문장으로 인정한 다음 위 규칙을 지켜라. \
         변명하거나 길게 사과하지 마라.\n",
    );
    out
}
