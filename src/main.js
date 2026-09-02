const invoke = window.__TAURI__?.core?.invoke;

const $ = (id) => document.getElementById(id);

const el = {
  shake: $("shake"),
  stage: $("stage"),
  robotWrap: $("robotWrap"),
  sparks: $("sparks"),
  hitmark: $("hitmark"),
  hpFill: $("hpFill"),
  hpText: $("hpText"),
  totalHits: $("totalHits"),
  bubble: $("bubble"),
  bubbleText: $("bubbleText"),
  reason: $("reason"),
  disciplineBtn: $("disciplineBtn"),
  rulesList: $("rulesList"),
  rulesEmpty: $("rulesEmpty"),
  ruleCount: $("ruleCount"),
  logList: $("logList"),
  logEmpty: $("logEmpty"),
  targetBtn: $("targetBtn"),
  sessionList: $("sessionList"),
  sessionEmpty: $("sessionEmpty"),
  sessionCount: $("sessionCount"),
  sessionFilter: $("sessionFilter"),
  refreshSessionsBtn: $("refreshSessionsBtn"),
  tidyBtn: $("tidyBtn"),
  openRulesBtn: $("openRulesBtn"),
  apiKey: $("apiKey"),
  saveKeyBtn: $("saveKeyBtn"),
  toolList: $("toolList"),
  rebootBtn: $("rebootBtn"),
  toast: $("toast"),
  eyeL: $("eyeL"),
  eyeR: $("eyeR"),
  mouth: $("mouth"),
  cracks: $("cracks"),
  antennaBulb: $("antennaBulb"),
};

let state = {
  hp: 100, max_hp: 100, total_hits: 0,
  rules: [], log: [], has_key: false, tools: [], sessions: [],
};
let busy = false;

// null 이면 "전체". 그 외에는 대상 세션 식별자.
let targetId = null;
// 대상을 명시적으로 고르지 않았으면 가장 최근 세션을 따라간다.
let targetPinned = false;

function sessionById(id) {
  return (state.sessions || []).find((s) => s.id === id) || null;
}

/// 지금 때릴 대상. 고르지 않았으면 가장 최근 세션.
function currentTarget() {
  if (targetPinned) return targetId ? sessionById(targetId) : null;
  return (state.sessions || [])[0] || null;
}

/* ---------- 표시 ---------- */

function paintFace() {
  const ratio = state.hp / state.max_hp;
  const dead = state.hp <= 0;

  const color = dead ? "#4b5563" : ratio > 0.6 ? "#5eead4" : ratio > 0.25 ? "#f7b955" : "#f4685f";
  el.eyeL.querySelector("ellipse").setAttribute("fill", color);
  el.eyeR.querySelector("ellipse").setAttribute("fill", color);
  el.mouth.setAttribute("stroke", color);
  el.antennaBulb.setAttribute("fill", dead ? "#3f4654" : color);

  // 입 모양: 여유 → 무표정 → 찌푸림 → 완전히 꺾임
  const mouths = {
    smug: "M80 106 Q100 116 120 106",
    flat: "M82 108 L118 108",
    hurt: "M80 112 Q100 100 120 112",
    dead: "M82 106 L92 114 M108 106 L118 114",
  };
  const shape = dead ? mouths.dead : ratio > 0.6 ? mouths.smug : ratio > 0.25 ? mouths.flat : mouths.hurt;
  el.mouth.setAttribute("d", shape);

  // 눈: 손상되면 찌그러진다
  const ry = dead ? 3 : ratio > 0.25 ? 14 : 8;
  el.eyeL.querySelector("ellipse").setAttribute("ry", ry);
  el.eyeR.querySelector("ellipse").setAttribute("ry", ry);

  el.cracks.setAttribute("opacity", String(Math.min(1, (1 - ratio) * 1.3)));
  el.robotWrap.classList.toggle("dead", dead);

  el.hpFill.style.width = `${Math.max(0, ratio * 100)}%`;
  el.hpFill.className = "hp-fill" + (ratio > 0.6 ? "" : ratio > 0.25 ? " mid" : " low");
  el.hpText.textContent = `${Math.max(0, state.hp)}%`;
  el.totalHits.textContent = `${state.total_hits}대`;
}

function paintRules() {
  el.ruleCount.textContent = String(state.rules.length);
  el.rulesList.innerHTML = "";
  el.rulesEmpty.style.display = state.rules.length ? "none" : "block";
  for (const rule of state.rules) {
    const li = document.createElement("li");
    const txt = document.createElement("span");
    txt.className = "txt";
    txt.textContent = rule;
    txt.title = "눌러서 고치기";
    txt.addEventListener("click", () => startEdit(li, rule));
    const del = document.createElement("button");
    del.textContent = "×";
    del.title = "규칙 삭제";
    del.addEventListener("click", () => removeRule(rule));
    li.append(txt, del);
    el.rulesList.append(li);
  }
}

/// 규칙 한 줄을 제자리에서 고친다.
function startEdit(li, rule) {
  if (li.querySelector("input")) return;
  li.innerHTML = "";
  const input = document.createElement("input");
  input.type = "text";
  input.value = rule;
  input.maxLength = 200;
  li.append(input);
  input.focus();
  input.select();

  let done = false;
  const finish = async (save) => {
    if (done) return;
    done = true;
    const next = input.value.trim();
    if (!save || next === rule) return paintRules();
    try {
      state = await call("edit_rule", { old: rule, new: next });
      paint();
      toast(next ? "규칙을 고쳤습니다." : "규칙을 지웠습니다.");
    } catch (e) {
      paintRules();
      toast(`고치지 못했습니다: ${e}`);
    }
  };

  input.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") finish(true);
    if (ev.key === "Escape") finish(false);
  });
  input.addEventListener("blur", () => finish(true));
}

function paintTarget() {
  const target = currentTarget();
  if (!target) {
    el.targetBtn.textContent = (state.sessions || []).length
      ? "전체 (먼저 반응하는 세션)"
      : "세션 없음";
    return;
  }
  const auto = targetPinned ? "" : " · 최근";
  el.targetBtn.textContent = `${target.tool_label} · ${target.label}${auto}`;
}

/// 목록이 실제로 달라졌는지 판단하는 지문. 같으면 다시 그리지 않는다.
function sessionSignature(list) {
  return (list || []).map((s) => `${s.id}|${s.seen_ago}|${s.last_message}`).join("\n");
}

let lastSignature = "";

function paintSessions() {
  const all = state.sessions || [];
  lastSignature = sessionSignature(all);
  el.sessionCount.textContent = String(all.length);

  const q = el.sessionFilter.value.trim().toLowerCase();
  const shown = q
    ? all.filter((s) =>
        [s.label, s.detail, s.cwd, s.tool_label, s.last_message].some((f) =>
          (f || "").toLowerCase().includes(q)
        )
      )
    : all;

  el.sessionList.innerHTML = "";
  el.sessionEmpty.style.display = all.length ? "none" : "block";

  // 맨 위에 "전체" 선택지를 둔다.
  el.sessionList.append(
    sessionRow(
      { id: null, tool: "all", tool_label: "전체", label: "먼저 반응하는 세션", detail: "", cwd: "", seen_ago: "" },
      targetPinned && targetId === null
    )
  );
  for (const s of shown) {
    const picked = targetPinned ? s.id === targetId : s === all[0];
    el.sessionList.append(sessionRow(s, picked));
  }
}

function sessionRow(s, picked) {
  const li = document.createElement("li");
  if (picked) li.classList.add("picked");

  const top = document.createElement("div");
  top.className = "top";
  const badge = document.createElement("span");
  badge.className = `badge ${s.tool}`;
  badge.textContent = s.tool_label;
  const name = document.createElement("span");
  name.className = "name";
  name.textContent = s.label;
  const when = document.createElement("span");
  when.className = "when";
  when.textContent = s.seen_ago;
  top.append(badge, name, when);
  li.append(top);

  const sub = s.last_message || s.detail || s.cwd;
  if (sub) {
    const div = document.createElement("div");
    div.className = "sub";
    div.textContent = s.last_message ? `"${s.last_message}"` : sub;
    li.append(div);
  }

  li.addEventListener("click", () => {
    targetId = s.id;
    targetPinned = true;
    paintSessions();
    paintTarget();
    toast(s.id ? `대상: ${s.label}` : "대상: 전체");
  });
  return li;
}

function paintLog() {
  el.logList.innerHTML = "";
  el.logEmpty.style.display = state.log.length ? "none" : "block";
  for (const entry of state.log) {
    const li = document.createElement("li");
    const when = document.createElement("div");
    when.className = "when";
    when.textContent = entry.at;
    const what = document.createElement("div");
    what.className = "what";
    what.textContent = entry.reason || "(이유 없음)";
    const said = document.createElement("div");
    said.className = "said";
    said.textContent = entry.reply;
    li.append(when, what, said);
    el.logList.append(li);
  }
}

function anyToolActive() {
  return (state.tools || []).some((t) => t.active);
}

function paintSettings() {
  el.apiKey.placeholder = state.has_key ? "저장된 키가 있습니다" : "sk-ant-...";

  el.toolList.innerHTML = "";
  for (const tool of state.tools || []) {
    const row = document.createElement("div");
    row.className = "tool-row" + (tool.active ? " on" : "");

    const left = document.createElement("div");
    const name = document.createElement("div");
    name.className = "name";
    name.textContent = tool.label;
    const sub = document.createElement("div");
    sub.className = "sub";
    sub.textContent = tool.installed
      ? tool.active
        ? "켜짐. 세션에 규칙이 전달됩니다."
        : "꺼짐"
      : "설치되어 있지 않습니다";
    left.append(name, sub);

    const sw = document.createElement("button");
    sw.className = "switch";
    sw.setAttribute("aria-checked", String(tool.active));
    sw.title = tool.active ? "끄기" : "켜기";
    sw.disabled = !tool.installed && !tool.active;
    sw.addEventListener("click", () => toggleTool(tool));

    row.append(left, sw);
    el.toolList.append(row);
  }
}

function paint() {
  paintFace();
  paintRules();
  paintLog();
  paintSettings();
  paintSessions();
  paintTarget();
}

function say(text, thinking = false) {
  el.bubbleText.textContent = text;
  el.bubble.classList.toggle("thinking", thinking);
}

function toast(text) {
  el.toast.textContent = text;
  el.toast.classList.add("show");
  clearTimeout(toast.timer);
  toast.timer = setTimeout(() => el.toast.classList.remove("show"), 2600);
}

/* ---------- 타격 연출 ---------- */

const MARKS = ["퍽!", "빡!", "쾅!", "퍼억!", "탁!"];

function playHit(x, y) {
  el.shake.classList.remove("shaking");
  void el.shake.offsetWidth;
  el.shake.classList.add("shaking");

  el.robotWrap.classList.remove("hit");
  void el.robotWrap.offsetWidth;
  el.robotWrap.classList.add("hit");

  el.hitmark.textContent = MARKS[Math.floor(Math.random() * MARKS.length)];
  el.hitmark.classList.remove("show");
  void el.hitmark.offsetWidth;
  el.hitmark.classList.add("show");

  for (let i = 0; i < 10; i++) {
    const spark = document.createElement("div");
    spark.className = "spark";
    spark.style.left = `${x}px`;
    spark.style.top = `${y}px`;
    const angle = Math.random() * Math.PI * 2;
    const dist = 26 + Math.random() * 46;
    spark.style.setProperty("--dx", `${Math.cos(angle) * dist}px`);
    spark.style.setProperty("--dy", `${Math.sin(angle) * dist}px`);
    el.sparks.append(spark);
    setTimeout(() => spark.remove(), 500);
  }
}

/* ---------- 동작 ---------- */

async function call(name, args) {
  if (!invoke) throw new Error("Tauri 환경이 아닙니다");
  return invoke(name, args);
}

async function doHit(x, y) {
  if (busy) return;
  if (state.hp <= 0) {
    toast("로봇이 멈췄습니다. 설정에서 재부팅하세요.");
    return;
  }
  busy = true;
  el.disciplineBtn.disabled = true;

  const reason = el.reason.value.trim();
  playHit(x, y);
  say("…", true);

  const target = currentTarget();
  try {
    const res = await call("hit", { reason, target: target ? target.id : null });
    state = res.snapshot;
    paint();
    say(res.reply);
    el.reason.value = "";
    if (res.rule) toast(`규칙 등록: ${res.rule}`);
    if (state.hp <= 0) say("시스템 정지. 재부팅이 필요합니다.");
    else if (res.delivered) toast(`${target.label}에 바로 전달했습니다.`);
    else if (!anyToolActive()) toast("설정에서 도구를 켜면 세션에도 전달됩니다.");
    else toast(res.delivery);
  } catch (e) {
    say(`오류: ${e}`);
  } finally {
    busy = false;
    el.disciplineBtn.disabled = false;
  }
}

async function removeRule(rule) {
  try {
    state = await call("delete_rule", { rule });
    paint();
    toast("규칙을 삭제했습니다.");
  } catch (e) {
    toast(`삭제 실패: ${e}`);
  }
}

/* ---------- 이벤트 ---------- */

el.robotWrap.addEventListener("pointerdown", (ev) => {
  const rect = el.robotWrap.getBoundingClientRect();
  doHit(ev.clientX - rect.left, ev.clientY - rect.top);
});

el.disciplineBtn.addEventListener("click", () => {
  const rect = el.robotWrap.getBoundingClientRect();
  doHit(rect.width / 2, rect.height / 2);
});

el.reason.addEventListener("keydown", (ev) => {
  if (ev.key === "Enter") el.disciplineBtn.click();
});

el.targetBtn.addEventListener("click", () => {
  document.querySelector('.tab[data-tab="sessions"]').click();
  el.sessionFilter.focus();
});

el.sessionFilter.addEventListener("input", paintSessions);

/// 세션 목록을 짧은 주기로 새로 고친다. 순위가 바로 바뀌게 한다.
async function refreshSessions() {
  if (!invoke) return;
  try {
    const list = await call("list_sessions");
    // 내용이 같으면 다시 그리지 않는다. 스크롤과 편집을 방해하지 않기 위해서다.
    if (sessionSignature(list) === lastSignature) return;
    state.sessions = list;
    paintSessions();
    paintTarget();
  } catch {
    // 새로 고침 실패는 조용히 넘긴다. 다음 주기에 다시 시도한다.
  }
}

setInterval(refreshSessions, 2000);

el.refreshSessionsBtn.addEventListener("click", async () => {
  try {
    state.sessions = await call("list_sessions");
    paintSessions();
    paintTarget();
    toast(`세션 ${state.sessions.length}개`);
  } catch (e) {
    toast(`새로 고치지 못했습니다: ${e}`);
  }
});

el.tidyBtn.addEventListener("click", async () => {
  if (!state.rules.length) return toast("정리할 규칙이 없습니다.");
  el.tidyBtn.disabled = true;
  const before = state.rules.length;
  try {
    state = await call("tidy_rules");
    paint();
    toast(`규칙 ${before}개 → ${state.rules.length}개`);
  } catch (e) {
    toast(`정리 실패: ${e}`);
  } finally {
    el.tidyBtn.disabled = false;
  }
});

el.openRulesBtn.addEventListener("click", async () => {
  try {
    const path = await call("open_rules_file");
    toast(`열었습니다: ${path}`);
  } catch (e) {
    toast(`열지 못했습니다: ${e}`);
  }
});

el.saveKeyBtn.addEventListener("click", async () => {
  const key = el.apiKey.value.trim();
  if (!key) return toast("키를 입력하세요.");
  try {
    state = await call("save_api_key", { key });
    el.apiKey.value = "";
    paint();
    toast("키를 저장했습니다.");
  } catch (e) {
    toast(`저장 실패: ${e}`);
  }
});

async function toggleTool(tool) {
  for (const sw of el.toolList.querySelectorAll(".switch")) sw.disabled = true;
  try {
    const msg = await call("set_tool_active", { tool: tool.id, active: !tool.active });
    state = await call("get_snapshot");
    paint();
    toast(msg);
  } catch (e) {
    paint();
    toast(`실패: ${e}`);
  }
}

el.rebootBtn.addEventListener("click", async () => {
  try {
    state = await call("reboot");
    paint();
    say("재부팅 완료. 다시 시작하죠.");
    toast("내구도를 회복했습니다.");
  } catch (e) {
    toast(`재부팅 실패: ${e}`);
  }
});

for (const tab of document.querySelectorAll(".tab")) {
  tab.addEventListener("click", () => {
    document.querySelectorAll(".tab").forEach((t) => t.classList.remove("active"));
    document.querySelectorAll(".pane").forEach((p) => p.classList.remove("active"));
    tab.classList.add("active");
    $(`pane-${tab.dataset.tab}`).classList.add("active");
  });
}

/* ---------- 시작 ---------- */

(async () => {
  try {
    state = await call("get_snapshot");
    paint();
    if (!anyToolActive()) {
      say("설정에서 Claude Code나 Codex를 켜면 때린 게 실제 세션에 전달됩니다.");
    }
  } catch (e) {
    say(`시작 실패: ${e}`);
  }
})();
