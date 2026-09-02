//! HitAI 훅 바이너리.
//!
//! 에이전트 세션 쪽에서 실행되어 HitAI의 훈육 결과를 실시간으로 반영한다.
//!
//! ```text
//! hitai-hook <이벤트> <도구>
//!   이벤트: session | prompt | tool
//!   도구:   claude | codex   (생략하면 claude)
//! ```
//!
//! - session : SessionStart. 누적된 훈육 규칙을 세션에 알린다.
//! - prompt  : UserPromptSubmit. 이 세션에 온 타격을 알린다.
//! - tool    : PreToolUse. 맞은 직후라면 진행 중인 도구 호출을 한 번 막는다.
//!
//! 표준 입력의 `session_id`로 이 세션을 겨냥한 타격만 골라 받는다. 세션 목록은 앱이
//! 대화 기록 파일에서 직접 읽으므로 훅이 등록해 줄 필요가 없다.
//!
//! 출력은 두 도구가 같은 JSON 규격을 쓴다. Codex는 평문을 오류로 처리하고,
//! Claude Code는 평문도 받지만 그러면 사용자 화면에 알릴 수단이 없다.
//!
//! 어떤 경우에도 세션을 망가뜨리지 않는다. 판단이 서지 않으면 조용히 통과시킨다.

use hitai_core::{rule_lines, State};
use serde_json::{json, Value};
use std::io::Read;

#[derive(Clone, Copy, PartialEq)]
enum Agent {
    ClaudeCode,
    Codex,
}

impl Agent {
    fn parse(name: &str) -> Agent {
        match name {
            "codex" => Agent::Codex,
            _ => Agent::ClaudeCode,
        }
    }

    /// 세션에 문맥을 덧붙인다.
    ///
    /// `additionalContext`는 모델만 보고 화면에는 나오지 않는다.
    /// 사용자도 알 수 있게 `systemMessage`를 함께 낸다. 두 도구 모두 같은 규격이다.
    fn emit_context(self, event: &str, text: &str, notice: &str) {
        let payload = json!({
            "hookSpecificOutput": {
                "hookEventName": event,
                "additionalContext": text,
            },
            "systemMessage": notice
        });
        println!("{payload}");
    }
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_default();
    let agent = Agent::parse(&args.next().unwrap_or_default());

    // 표준 입력은 항상 끝까지 읽는다. 파이프가 막히면 세션이 멈춘다.
    let mut raw = String::new();
    let _ = std::io::stdin().read_to_string(&mut raw);
    let payload: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);

    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    match mode.as_str() {
        "session" => on_session(agent),
        "prompt" => on_prompt(agent, &session_id),
        "tool" => on_tool(&session_id),
        _ => {}
    }
}

/// SessionStart: 세션이 시작할 때 누적 규칙을 알린다.
/// 규칙은 특정 세션의 것이 아니라 전체에 적용되므로 대상을 가리지 않는다.
fn on_session(agent: Agent) {
    let rules = rule_lines();
    if rules.is_empty() {
        return;
    }
    let state = State::load();
    let mut out = String::from(
        "[HitAI] 사용자가 HitAI 앱에서 너를 때리며 남긴 훈육 규칙이다. 이번 세션 내내 지켜라.\n\n",
    );
    for rule in &rules {
        out.push_str(&format!("- {rule}\n"));
    }
    out.push_str(&format!(
        "\n지금까지 맞은 횟수는 {}대다. 규칙을 어기면 또 맞는다.\n",
        state.total_hits
    ));
    agent.emit_context(
        "SessionStart",
        &out,
        &format!("HitAI: 훈육 규칙 {}개를 전달했습니다.", rules.len()),
    );
}

/// UserPromptSubmit: 이 세션에 온 타격을 알린다.
fn on_prompt(agent: Agent, session_id: &str) {
    let mut state = State::load();
    let pending: Vec<_> = state
        .pending_for(session_id)
        .into_iter()
        .cloned()
        .collect();
    if pending.is_empty() {
        return;
    }

    let mut out = String::new();
    out.push_str("[HitAI] 사용자가 방금 HitAI 앱에서 너를 때렸다. ");
    out.push_str(&format!(
        "지금까지 맞은 횟수는 {}대이고, 이번에 맞은 이유는 아래와 같다.\n\n",
        state.total_hits
    ));

    for hit in &pending {
        let when = hit.at.with_timezone(&chrono::Local).format("%H:%M");
        let reason = if hit.reason.trim().is_empty() {
            "(이유를 적지 않았다)"
        } else {
            hit.reason.trim()
        };
        out.push_str(&format!("- {when} · {reason}\n"));
        if let Some(rule) = &hit.rule {
            out.push_str(&format!("  → 규칙으로 등록됨: {rule}\n"));
        }
    }

    let rules = rule_lines();
    if !rules.is_empty() {
        out.push_str("\n지금까지 누적된 훈육 규칙 전체:\n");
        for rule in &rules {
            out.push_str(&format!("- {rule}\n"));
        }
    }

    out.push_str(
        "\n먼저 무엇을 잘못했는지 한 문장으로 인정하고, 위 규칙을 이번 응답부터 지켜라. \
         변명하거나 길게 사과하지 마라.\n",
    );

    let reason = pending
        .iter()
        .rev()
        .map(|h| h.reason.trim())
        .find(|r| !r.is_empty())
        .unwrap_or("이유 없음");
    let notice = format!(
        "HitAI: {}대 맞았습니다 (누적 {}대). {reason}",
        pending.len(),
        state.total_hits
    );
    agent.emit_context("UserPromptSubmit", &out, &notice);

    // 이 세션에 전달한 것만 소모한다. 다른 세션을 겨냥한 타격은 남겨 둔다.
    for hit in state.hits.iter_mut() {
        if !hit.injected && hit.targets(session_id) {
            hit.injected = true;
        }
    }
    let _ = state.save();
}

/// PreToolUse: 이 세션을 겨냥한 타격이 아직 살아 있으면 도구 호출을 거부한다.
///
/// 거부 규격은 두 도구가 같다. Codex는 빈 이유를 거부하므로 항상 이유를 채운다.
fn on_tool(session_id: &str) {
    let mut state = State::load();
    let Some(hot) = state.hot_for(session_id) else {
        return;
    };
    let reason = hot.reason.trim().to_string();

    let mut message = String::from(
        "[HitAI] 사용자가 방금 너를 때렸다. 하던 작업을 여기서 멈춰라. \
         이 도구 호출은 취소되었다.",
    );
    if !reason.is_empty() {
        message.push_str(&format!(" 맞은 이유: {reason}"));
    }
    message.push_str(
        " 도구를 다시 호출하지 말고, 무엇을 잘못했는지 인정한 다음 \
         어떻게 다르게 할지 사용자에게 먼저 확인해라.",
    );

    let payload = json!({
        "hookSpecificOutput": {
            "hookEventName": "PreToolUse",
            "permissionDecision": "deny",
            "permissionDecisionReason": message,
        },
        "systemMessage": "HitAI: 훈육으로 도구 호출이 차단되었습니다."
    });
    println!("{payload}");

    // 몇 대를 맞았든 작업은 한 번만 끊는다. 대기 중인 나머지 타격도 함께 소모한다.
    // 그러지 않으면 연속으로 때린 만큼 도구 호출이 줄줄이 막혀 세션이 멈춘다.
    for hit in state.hits.iter_mut() {
        if hit.targets(session_id) {
            hit.blocked = true;
        }
    }
    let _ = state.save();
}
