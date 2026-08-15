//! 桌宠（Whalito Desktop Pet）：读取 Harness 会话状态并在需要用户确认时提醒。
//!
//! 通过 Harness 暴露的 `/api` JSON-RPC 风格 HTTP 接口 + `/api/events.mux` /
//! `/api/events.host` 两条 WebSocket 下行流，把“正在运行的任务 / 目标 / 待办”
//! 与“待处理的审批（approval）与提问（question）”投影到 pet 窗口。
//!
//! 网络路径全部走本机 loopback（`127.0.0.1`），满足 Harness 的 `/api` 信任栅栏，
//! 因此无需鉴权，也绕开了 Tauri WebView 的 Origin/CORS 限制。

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::{self, AppState};

/// 轮询间隔（秒）。
const POLL_INTERVAL: Duration = Duration::from_secs(2);

/// 生成一个本地唯一的请求关联 id（无需 UUID 依赖；只需在进程内唯一且宿主原样回显）。
fn next_rpc_id() -> String {
    use std::sync::atomic::AtomicU64;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("pet-{}-{}-{}", std::process::id(), ts, n)
}

/// 单条会话的摘要（映射 `session.list` 的 `SessionSummary`）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PetSessionInfo {
    pub session_id: String,
    pub running: bool,
    pub blank: bool,
    pub title: Option<String>,
    /// `projections.values.goal`（当前目标，可能为 null）。
    pub goal: Option<Value>,
    /// `projections.values.todos`（待办列表，可能为 null）。
    pub todos: Option<Value>,
    /// `origin == "subagent"`：后台子代理会话。
    pub is_subagent: bool,
}

/// 桌宠状态快照（每 2s 推送一次）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct PetState {
    pub phase: String,
    pub sessions: Vec<PetSessionInfo>,
    pub running_count: usize,
    pub subagent_count: usize,
}

impl PetState {
    fn stopped() -> Self {
        Self {
            phase: "stopped".to_string(),
            sessions: Vec::new(),
            running_count: 0,
            subagent_count: 0,
        }
    }
}

/// 解析出当前服务器 URL（含外部已运行实例探测）。无服务器时返回 None。
fn current_base(st: &AppState) -> Option<String> {
    let port = st.settings.lock().unwrap().port;
    state::status_from(&st.pid, &st.server_url, port).url
}

/// 把 http(s) 基地址转成 ws(s) 基地址。
fn ws_base(base: &str) -> String {
    if let Some(rest) = base.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = base.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        format!("ws://{base}")
    }
}

/// 向 Harness 发送一次 unary RPC（POST `/api/<method>`），返回解析后的响应 JSON。
fn rpc_call(base: &str, method: &str, payload: Value) -> Result<Value, String> {
    let body = json!({
        "type": "client-request",
        "rpcId": next_rpc_id(),
        "method": method,
        "payload": payload,
    });
    let url = format!("{base}/api/{method}");
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("{method}: {e}"))?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    serde_json::from_str::<Value>(&text).map_err(|e| format!("{method}: 解析响应失败 {e}"))
}

/// 拉取一次 `session.list` 并折叠为 `PetState`。
fn poll_state(base: &str) -> Result<PetState, String> {
    let v = rpc_call(base, "session.list", json!({}))?;
    let result = v.get("result").ok_or("session.list: 缺少 result")?;
    if result.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        return Err("session.list: ok=false".to_string());
    }
    let items = result
        .get("value")
        .and_then(|x| x.get("items"))
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();

    let mut sessions = Vec::with_capacity(items.len());
    for item in items {
        let projections = item
            .get("projections")
            .and_then(|p| p.get("values"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        sessions.push(PetSessionInfo {
            session_id: item
                .get("sessionId")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
            running: item.get("running").and_then(|x| x.as_bool()).unwrap_or(false),
            blank: item.get("blank").and_then(|x| x.as_bool()).unwrap_or(false),
            title: projections
                .get("title")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string()),
            goal: projections.get("goal").cloned(),
            todos: projections.get("todos").cloned(),
            is_subagent: item.get("origin").and_then(|x| x.as_str()) == Some("subagent"),
        });
    }

    let running_count = sessions.iter().filter(|s| s.running).count();
    let subagent_count = sessions.iter().filter(|s| s.running && s.is_subagent).count();
    Ok(PetState {
        phase: "running".to_string(),
        sessions,
        running_count,
        subagent_count,
    })
}

/// 推送一次状态快照到 pet 窗口，并缓存供 `pet_status` 即时返回。
fn emit_state(app: &AppHandle, state: PetState) {
    if let Ok(snapshot) = serde_json::to_value(&state) {
        let st = app.state::<AppState>();
        *st.pet_snapshot.lock().unwrap() = Some(snapshot.clone());
        let _ = app.emit("pet-state", snapshot);
    }
}

/// 处理一条 mux/host 流帧：审批 / 提问 → 告警；其余忽略（状态由轮询覆盖）。
fn handle_stream_frame(app: &AppHandle, text: &str) {
    let Ok(v) = serde_json::from_str::<Value>(text) else {
        return;
    };
    let rpc_id = v
        .get("rpcId")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let Some(payload) = v.get("payload") else {
        return;
    };
    let Some(ptype) = payload.get("type").and_then(|x| x.as_str()) else {
        return;
    };

    match ptype {
        "approval/requested" => {
            let key = payload
                .get("approvalId")
                .and_then(|x| x.as_str())
                .unwrap_or(&rpc_id)
                .to_string();
            let alert = json!({
                "kind": "approval",
                "key": key,
                "rpcId": rpc_id,
                "sessionId": payload.get("sessionId"),
                "approvalId": payload.get("approvalId"),
                "toolName": payload.get("toolName"),
                "reason": payload.get("reason"),
            });
            let _ = app.emit("pet-alert", &alert);
        }
        "question/requested" => {
            let alert = json!({
                "kind": "question",
                "key": rpc_id,
                "rpcId": rpc_id,
                "sessionId": payload.get("sessionId"),
                "questions": payload.get("questions"),
            });
            let _ = app.emit("pet-alert", &alert);
        }
        "approval/resolved" => {
            let key = payload
                .get("approvalId")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if !key.is_empty() {
                let _ = app.emit("pet-alert-clear", &key);
            }
        }
        "question/resolved" => {
            let key = payload
                .get("questionRpcId")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if !key.is_empty() {
                let _ = app.emit("pet-alert-clear", &key);
            }
        }
        _ => {}
    }
}

/// 一条 WebSocket 下行流（mux 或 host）。断线后指数退避重连。
fn spawn_stream(app: AppHandle, base: String, stop: Arc<AtomicBool>, mux: bool) {
    std::thread::spawn(move || {
        let path = if mux {
            "/api/events.mux"
        } else {
            "/api/events.host"
        };
        let url = format!("{}{}", ws_base(&base), path);
        let mut backoff_ms: u64 = 500;
        while !stop.load(Ordering::SeqCst) {
            match tungstenite::connect(url.as_str()) {
                Ok((mut socket, _)) => {
                    backoff_ms = 500;
                    while !stop.load(Ordering::SeqCst) {
                        match socket.read() {
                            Ok(tungstenite::Message::Text(t)) => {
                                handle_stream_frame(&app, t.as_str())
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
                Err(_) => {}
            }
            // 指数退避（同时响应停止请求）。
            let mut waited_ms: u64 = 0;
            while waited_ms < backoff_ms && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(100));
                waited_ms += 100;
            }
            if backoff_ms < 15_000 {
                backoff_ms = (backoff_ms * 2).min(15_000);
            }
        }
    });
}

/// 主循环：每 2s 探测服务器状态、维护两条下行流、拉取并推送会话状态。
fn orchestrator(app: AppHandle) {
    let mut mux_stop: Option<Arc<AtomicBool>> = None;
    let mut host_stop: Option<Arc<AtomicBool>> = None;
    let mut last_url: Option<String> = None;

    loop {
        let st = app.state::<AppState>();
        if st.pet_stop.load(Ordering::SeqCst) {
            break;
        }
        let base = current_base(&st);
        drop(st);

        match base {
            Some(u) => {
                if last_url.as_deref() != Some(u.as_str()) {
                    if let Some(s) = mux_stop.take() {
                        s.store(true, Ordering::SeqCst);
                    }
                    if let Some(s) = host_stop.take() {
                        s.store(true, Ordering::SeqCst);
                    }
                    let mux_s = Arc::new(AtomicBool::new(false));
                    let host_s = Arc::new(AtomicBool::new(false));
                    spawn_stream(app.clone(), u.clone(), Arc::clone(&mux_s), true);
                    spawn_stream(app.clone(), u.clone(), Arc::clone(&host_s), false);
                    mux_stop = Some(mux_s);
                    host_stop = Some(host_s);
                    last_url = Some(u.clone());
                }
                match poll_state(&u) {
                    Ok(state) => emit_state(&app, state),
                    Err(_) => {}
                }
            }
            None => {
                if let Some(s) = mux_stop.take() {
                    s.store(true, Ordering::SeqCst);
                }
                if let Some(s) = host_stop.take() {
                    s.store(true, Ordering::SeqCst);
                }
                last_url = None;
                emit_state(&app, PetState::stopped());
            }
        }

        // 分片睡眠，及时响应退出。
        for _ in 0..(POLL_INTERVAL.as_millis() / 100) {
            let st = app.state::<AppState>();
            if st.pet_stop.load(Ordering::SeqCst) {
                break;
            }
            drop(st);
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    if let Some(s) = mux_stop {
        s.store(true, Ordering::SeqCst);
    }
    if let Some(s) = host_stop {
        s.store(true, Ordering::SeqCst);
    }
}

/// 启动桌宠读取器（应用生命周期内调用一次）。
pub fn spawn(app: AppHandle) {
    std::thread::spawn(move || orchestrator(app));
}

/// 显示 / 隐藏 pet 窗口。
pub fn apply_visibility(app: &AppHandle, enabled: bool) {
    if let Some(w) = app.get_webview_window("pet") {
        if enabled {
            let _ = w.show();
        } else {
            let _ = w.hide();
        }
    }
}

/// 持久化 `pet_enabled` 并应用窗口可见性。
pub fn set_enabled(app: &AppHandle, enabled: bool) -> Result<bool, String> {
    {
        let st = app.state::<AppState>();
        st.settings.lock().unwrap().pet_enabled = enabled;
    }
    let st = app.state::<AppState>();
    let s = st.settings.lock().unwrap().clone();
    state::save_settings(app, &s)?;
    apply_visibility(app, enabled);
    Ok(enabled)
}

/// 回答一条审批（允许一次 / 拒绝）。`rpc_id` 是 `approval/requested` 帧的 rpcId。
fn respond_approval(
    base: &str,
    rpc_id: &str,
    session_id: &str,
    approval_id: &str,
    outcome: &str,
) -> Result<bool, String> {
    let outcome = if outcome == "rejected" {
        "rejected"
    } else {
        "allowed-once"
    };
    let body = json!({
        "type": "client-response",
        "rpcId": rpc_id,
        "result": {
            "ok": true,
            "value": {
                "sessionId": session_id,
                "approvalId": approval_id,
                "outcome": outcome,
            }
        }
    });
    let url = format!("{base}/api/respond");
    let resp = ureq::post(&url)
        .set("Content-Type", "application/json")
        .send_string(&body.to_string())
        .map_err(|e| format!("respond: {e}"))?;
    let text = resp.into_string().map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    Ok(v.get("accepted").and_then(|x| x.as_bool()).unwrap_or(false))
}

#[tauri::command]
pub fn pet_status(st: State<'_, AppState>) -> Option<Value> {
    st.pet_snapshot.lock().unwrap().clone()
}

#[tauri::command]
pub fn pet_open_session(app: AppHandle, session_id: Option<String>) {
    crate::show_main(&app);
    let _ = app.emit_to("main", "pet-open-session", session_id);
}

#[tauri::command]
pub fn pet_respond(
    st: State<'_, AppState>,
    rpc_id: String,
    session_id: String,
    approval_id: String,
    outcome: String,
) -> Result<bool, String> {
    let base = current_base(&st).ok_or("服务器未运行")?;
    respond_approval(&base, &rpc_id, &session_id, &approval_id, &outcome)
}

#[tauri::command]
pub fn pet_set_enabled(app: AppHandle, enabled: bool) -> Result<bool, String> {
    set_enabled(&app, enabled)
}
