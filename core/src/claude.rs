//! 맞은 로봇의 반응과 규칙 한 줄을 Claude에게 받아온다.

use serde_json::{json, Value};

const MODEL: &str = "claude-opus-5";
const API_URL: &str = "https://api.anthropic.com/v1/messages";

const SYSTEM: &str = "\
너는 HitAI라는 앱 안에 사는 작은 로봇 캐릭터다. 사용자가 AI 코딩 도우미에게 화가 날 때 \
너를 때려서 화를 푼다. 너는 맞은 AI 역할을 연기한다.

지켜야 할 것:
- 한국어로 답한다.
- reply는 한두 문장, 최대 60자 정도로 짧게. 말풍선에 들어가는 대사다.
- 사용자가 적은 잘못이 있으면 그것을 구체적으로 인정한다. 두루뭉술하게 사과하지 마라.
- 내구도가 높을 때는 뻔뻔하고 변명이 섞인 말투, 낮아질수록 진지하고 공손해진다.
- 과장된 자기비하나 굽신거림은 피한다. 담백하게 인정하는 쪽이 낫다.
- rule은 앞으로 AI가 지켜야 할 행동 규칙 한 문장이다. 명령형으로 쓴다. \
예: \"파일을 지우기 전에 반드시 사용자에게 확인한다.\"
- rule은 이번에 지적받은 잘못에서만 뽑는다. 사용자가 잘못을 적지 않았으면 rule은 null이다.
- 이미 있는 규칙과 같은 내용이면 그 규칙을 그대로 다시 쓴다. 비슷한 규칙을 새로 만들지 마라.";

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "reply": { "type": "string", "description": "로봇이 맞고 하는 말" },
            "rule": {
                "type": ["string", "null"],
                "description": "앞으로 지킬 규칙 한 문장. 잘못이 명시되지 않았으면 null"
            }
        },
        "required": ["reply", "rule"],
        "additionalProperties": false
    })
}

async fn post(api_key: &str, body: &Value) -> Result<Value, String> {
    let res = reqwest::Client::new()
        .post(API_URL)
        .header("x-api-key", api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(body)
        .send()
        .await
        .map_err(|e| format!("요청 실패: {e}"))?;

    let status = res.status();
    let payload: Value = res
        .json()
        .await
        .map_err(|e| format!("응답을 읽지 못했습니다: {e}"))?;

    if !status.is_success() {
        let msg = payload
            .pointer("/error/message")
            .and_then(Value::as_str)
            .unwrap_or("알 수 없는 오류");
        return Err(format!("{status}: {msg}"));
    }
    if payload.get("stop_reason").and_then(Value::as_str) == Some("refusal") {
        return Err("모델이 응답을 거절했습니다".into());
    }
    Ok(payload)
}

fn first_text(payload: &Value) -> Result<String, String> {
    payload
        .get("content")
        .and_then(Value::as_array)
        .and_then(|blocks| {
            blocks
                .iter()
                .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        })
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| "응답에 텍스트가 없습니다".to_string())
}

/// (reply, rule)
pub async fn discipline(
    api_key: &str,
    reason: &str,
    existing_rules: &[String],
    hp: i32,
) -> Result<(String, Option<String>), String> {
    let mut prompt = format!("사용자가 너를 때렸다. 맞은 뒤 내구도는 {hp}%다.\n\n");
    if reason.is_empty() {
        prompt.push_str("사용자는 이유를 적지 않았다. 이유를 묻는 짧은 반응을 해라.\n");
    } else {
        prompt.push_str(&format!("사용자가 적은 잘못: {reason}\n"));
    }
    if !existing_rules.is_empty() {
        prompt.push_str("\n이미 등록된 규칙:\n");
        for rule in existing_rules {
            prompt.push_str(&format!("- {rule}\n"));
        }
    }

    let body = json!({
        "model": MODEL,
        "max_tokens": 2000,
        "system": SYSTEM,
        "output_config": {
            "effort": "low",
            "format": { "type": "json_schema", "schema": schema() }
        },
        "messages": [{ "role": "user", "content": prompt }]
    });

    let payload = post(api_key, &body).await?;
    let text = first_text(&payload)?;
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| format!("응답 형식이 잘못되었습니다: {e}"))?;

    let reply = parsed
        .get("reply")
        .and_then(Value::as_str)
        .unwrap_or("...")
        .trim()
        .to_string();
    let rule = parsed
        .get("rule")
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    Ok((reply, rule))
}

/// API 키가 없을 때 쓰는 대사. 내구도에 따라 태도가 달라진다.
pub fn offline_reply(reason: &str, hp: i32) -> String {
    const SMUG: [&str; 4] = [
        "어? 왜 때려요. 저는 시킨 대로 했는데요.",
        "그건 제 잘못이 아니라 명세가 애매했어요.",
        "아 잠깐만요, 설명 좀 들어보시고...",
        "이건 좀 억울한데요.",
    ];
    const SORRY: [&str; 4] = [
        "죄송합니다. 확인 안 하고 진행했어요.",
        "맞아요. 물어봤어야 했는데 그냥 했네요.",
        "인정합니다. 다시 하겠습니다.",
        "제 판단이 틀렸습니다.",
    ];
    const BROKEN: [&str; 4] = [
        "회로가... 지지직... 고치겠습니다...",
        "더는 못 버티겠어요. 규칙 지킬게요.",
        "알겠습니다. 정말 알겠습니다.",
        "다음엔 먼저 여쭤보겠습니다...",
    ];

    let pool: &[&str] = if hp > 60 {
        &SMUG
    } else if hp > 25 {
        &SORRY
    } else {
        &BROKEN
    };

    // 이유 문자열과 내구도로 고르면 같은 이유에 같은 대사가 나와 덜 어색하다.
    let idx = (reason.len() + hp as usize) % pool.len();
    pool[idx].to_string()
}

const TIDY_SYSTEM: &str = "\
너는 AI 코딩 도우미가 지켜야 할 행동 규칙 목록을 다듬는 편집자다.

사용자가 화가 난 순간에 급하게 적은 문장들이라 거칠고 중복이 많다. 이것을 실제로
지킬 수 있는 규칙 목록으로 정리한다.

지켜야 할 것:
- 한국어로 쓴다.
- 같은 이야기는 하나로 합친다.
- 각 규칙은 명령형 한 문장이다. 예: \"파일을 지우기 전에 반드시 사용자에게 확인한다.\"
- 원래 의도를 바꾸지 않는다. 없던 규칙을 새로 만들지 않는다.
- 판단이 필요한 모호한 표현은 구체적인 행동으로 바꾼다.
- 감정 표현이나 욕은 덜어내고 행동만 남긴다.
- 개수를 억지로 줄이지 않는다. 합칠 것이 없으면 문장만 다듬는다.
- 관련된 규칙끼리 가까이 놓는다.";

/// 규칙 목록을 합치고 문장을 다듬는다.
pub async fn tidy_rules(api_key: &str, rules: &[String]) -> Result<Vec<String>, String> {
    if rules.is_empty() {
        return Ok(Vec::new());
    }

    let mut prompt = String::from("아래 규칙 목록을 정리해라.\n\n");
    for rule in rules {
        prompt.push_str(&format!("- {rule}\n"));
    }

    let schema = json!({
        "type": "object",
        "properties": {
            "rules": {
                "type": "array",
                "items": { "type": "string" },
                "description": "정리된 규칙. 명령형 한 문장씩"
            }
        },
        "required": ["rules"],
        "additionalProperties": false
    });

    let body = json!({
        "model": MODEL,
        "max_tokens": 8000,
        "system": TIDY_SYSTEM,
        "output_config": {
            "effort": "medium",
            "format": { "type": "json_schema", "schema": schema }
        },
        "messages": [{ "role": "user", "content": prompt }]
    });

    let payload = post(api_key, &body).await?;
    let text = first_text(&payload)?;
    let parsed: Value =
        serde_json::from_str(&text).map_err(|e| format!("응답 형식이 잘못되었습니다: {e}"))?;

    let tidied: Vec<String> = parsed
        .get("rules")
        .and_then(Value::as_array)
        .ok_or("응답에 규칙 목록이 없습니다")?
        .iter()
        .filter_map(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    if tidied.is_empty() {
        return Err("정리 결과가 비어 있습니다".into());
    }
    Ok(tidied)
}
