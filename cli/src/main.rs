//! HitAI CLI. 앱을 열지 않고도 때리고 연동을 켜고 끌 수 있다.

use hitai_core::link::{self, Tool, ALL_TOOLS};
use hitai_core::{sessions, Config, State, MAX_HP};

const USAGE: &str = "\
HitAI - 말 안 듣는 AI를 때려서 훈육한다

사용법:
  hitai hit [이유]            한 대 때린다. 이유를 적으면 규칙으로 남는다
                              --to <세션> 으로 대상을 고른다 (기본: 가장 최근 세션)
                              --all 로 먼저 반응하는 세션에 보낸다
  hitai sessions              최근 대화한 세션 목록
  hitai on [도구]             연동을 켠다 (claude | codex | all, 기본값 all)
  hitai off [도구]            연동을 끈다. HitAI가 넣은 설정만 지우고 백업한다
  hitai status                내구도, 누적 타격, 규칙 수, 도구별 연동 상태
  hitai rules                 등록된 규칙을 모두 보여준다
  hitai forget <규칙>         규칙 하나를 지운다
  hitai edit                  규칙 파일을 편집기로 연다 ($EDITOR)
  hitai tidy                  AI가 규칙을 합치고 문장을 다듬는다 (API 키 필요)
  hitai reboot                내구도를 회복한다
  hitai key <API 키>          Anthropic API 키를 저장한다 (없어도 동작한다)

예시:
  hitai hit \"묻지도 않고 파일 지움\"
  hitai hit \"테스트 지웠음\" --to tesla
  hitai off codex
";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("");

    let result = match cmd {
        "hit" => cmd_hit(&args[1..]),
        "sessions" => cmd_sessions(),
        "on" | "link" | "activate" => cmd_toggle(&args[1..], true),
        "off" | "unlink" | "deactivate" => cmd_toggle(&args[1..], false),
        "status" => cmd_status(),
        "rules" => cmd_rules(),
        "forget" => cmd_forget(&args[1..]),
        "edit" => cmd_edit(),
        "tidy" => cmd_tidy(),
        "reboot" => cmd_reboot(),
        "key" => cmd_key(&args[1..]),
        "help" | "--help" | "-h" | "" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("모르는 명령입니다: {other}\n\n{USAGE}")),
    };

    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn cmd_hit(args: &[String]) -> Result<(), String> {
    // --to <세션>, --all 을 걸러내고 남은 말을 이유로 쓴다.
    let mut reason_parts: Vec<&str> = Vec::new();
    let mut wanted: Option<String> = None;
    let mut broadcast = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--to" => {
                wanted = args.get(i + 1).cloned();
                if wanted.is_none() {
                    return Err("--to 뒤에 세션을 적어 주세요. `hitai sessions`로 확인할 수 있습니다.".into());
                }
                i += 2;
            }
            "--all" => {
                broadcast = true;
                i += 1;
            }
            other => {
                reason_parts.push(other);
                i += 1;
            }
        }
    }
    let reason = reason_parts.join(" ");

    // 대상을 정한다. 지정하지 않으면 가장 최근에 대화한 세션이다.
    let session = if broadcast {
        None
    } else {
        match &wanted {
            Some(name) => Some(sessions::resolve(name).ok_or_else(|| {
                format!("{name} 에 맞는 세션이 없습니다. `hitai sessions`로 확인해 주세요.")
            })?),
            None => sessions::latest(),
        }
    };
    let target_label = match &session {
        Some(s) => format!("[{}] {} ({})", s.tool_label, s.label, s.seen_ago),
        None => "전체 (먼저 반응하는 세션)".to_string(),
    };
    let target = session.map(|s| s.id);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let outcome = rt.block_on(hitai_core::hit(&reason, target))?;

    println!("퍽!  내구도 {}%  누적 {}대", outcome.state.hp, outcome.state.total_hits);
    println!("대상: {target_label}");
    println!("전달: {}", outcome.delivery.describe());
    println!("로봇: {}", outcome.reply);
    match &outcome.rule {
        Some(rule) => println!("규칙 등록: {rule}"),
        None => println!("(이유를 적으면 규칙으로 남습니다)"),
    }
    if !link::any_active() {
        println!("\n연동된 도구가 없습니다. `hitai on`으로 켜면 세션에 전달됩니다.");
    }
    Ok(())
}

fn cmd_sessions() -> Result<(), String> {
    let all = sessions::list();
    if all.is_empty() {
        println!("등록된 세션이 없습니다.");
        println!("연동을 켜고 세션에서 한 번 대화하면 여기에 잡힙니다.");
        return Ok(());
    }
    println!("최근 대화한 세션 (최근 순)");
    for s in all {
        let branch = if s.detail.is_empty() {
            String::new()
        } else {
            format!("  ({})", s.detail)
        };
        println!("  [{:<11}] {}{branch}   {}", s.tool_label, s.label, s.seen_ago);
        if !s.last_message.is_empty() {
            println!("  {:<13}  \"{}\"", "", s.last_message);
        }
    }
    Ok(())
}

fn cmd_edit() -> Result<(), String> {
    let path = hitai_core::rules_path().map_err(|e| e.to_string())?;
    // 파일이 없으면 편집기가 빈 파일을 열지 않도록 미리 만들어 둔다.
    if !path.exists() {
        hitai_core::set_rules(&[]).map_err(|e| e.to_string())?;
    }
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".to_string());

    let status = std::process::Command::new(&editor)
        .arg(&path)
        .status()
        .map_err(|e| format!("{editor} 실행 실패: {e}"))?;
    if !status.success() {
        return Err(format!("{editor}가 정상 종료하지 않았습니다"));
    }
    // 편집 결과를 정규화하고 연동된 도구의 규칙 블록을 갱신한다.
    let rules = hitai_core::rule_lines();
    hitai_core::set_rules(&rules).map_err(|e| e.to_string())?;
    println!("규칙 {}개", rules.len());
    Ok(())
}

fn cmd_tidy() -> Result<(), String> {
    let key = Config::load().api_key.trim().to_string();
    if key.is_empty() {
        return Err("API 키가 필요합니다. `hitai key sk-ant-...`로 저장해 주세요.".into());
    }
    let before = hitai_core::rule_lines();
    if before.is_empty() {
        println!("정리할 규칙이 없습니다.");
        return Ok(());
    }

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    let after = rt.block_on(hitai_core::claude::tidy_rules(&key, &before))?;

    hitai_core::set_rules(&after).map_err(|e| e.to_string())?;
    println!("규칙 {}개 → {}개\n", before.len(), after.len());
    for rule in &after {
        println!("- {rule}");
    }
    Ok(())
}

fn cmd_toggle(args: &[String], on: bool) -> Result<(), String> {
    let target = args.first().map(String::as_str).unwrap_or("all");
    let tools: Vec<Tool> = if target == "all" {
        ALL_TOOLS.to_vec()
    } else {
        vec![Tool::parse(target)
            .ok_or_else(|| format!("모르는 도구입니다: {target} (claude, codex, all 중 하나)"))?]
    };

    for tool in tools {
        // all을 지정했을 때는 설치되지 않은 도구를 조용히 건너뛴다.
        if on && target == "all" && !tool.is_installed() {
            println!("{}: 설치되어 있지 않아 건너뜁니다", tool.label());
            continue;
        }
        let msg = if on {
            link::activate(tool)
        } else {
            link::deactivate(tool)
        }?;
        println!("{msg}");
    }
    Ok(())
}

fn cmd_status() -> Result<(), String> {
    let state = State::load();
    let rules = hitai_core::rule_lines();
    println!("내구도   {}% / {}%", state.hp, MAX_HP);
    println!("누적     {}대", state.total_hits);
    println!("규칙     {}개", rules.len());
    println!("세션     {}개", sessions::list().len());
    println!(
        "API 키   {}",
        if Config::load().api_key.trim().is_empty() {
            "없음 (미리 준비된 대사로 동작)"
        } else {
            "저장됨"
        }
    );
    println!("\n연동 상태");
    for s in link::status() {
        let mark = if s.active { "켜짐" } else { "꺼짐" };
        let note = if s.installed { "" } else { "  (설치되지 않음)" };
        println!("  {:<12} {mark}{note}", s.label);
    }
    Ok(())
}

fn cmd_rules() -> Result<(), String> {
    let rules = hitai_core::rule_lines();
    if rules.is_empty() {
        println!("등록된 규칙이 없습니다.");
        return Ok(());
    }
    for rule in rules {
        println!("- {rule}");
    }
    Ok(())
}

fn cmd_forget(args: &[String]) -> Result<(), String> {
    let rule = args.join(" ");
    if rule.trim().is_empty() {
        return Err("지울 규칙을 적어 주세요.".into());
    }
    hitai_core::remove_rule(&rule).map_err(|e| e.to_string())?;
    println!("지웠습니다: {rule}");
    Ok(())
}

fn cmd_reboot() -> Result<(), String> {
    let mut state = State::load();
    state.reboot();
    state.save().map_err(|e| e.to_string())?;
    println!("재부팅 완료. 내구도 {}%", state.hp);
    Ok(())
}

fn cmd_key(args: &[String]) -> Result<(), String> {
    let key = args.join(" ").trim().to_string();
    if key.is_empty() {
        return Err("API 키를 함께 적어 주세요.".into());
    }
    Config { api_key: key }.save().map_err(|e| e.to_string())?;
    println!("API 키를 저장했습니다.");
    Ok(())
}
