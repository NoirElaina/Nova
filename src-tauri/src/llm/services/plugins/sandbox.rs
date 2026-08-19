// 插件 JS 沙箱：基于 boa_engine（纯 Rust JS 引擎）的专用执行线程。
//
// 设计要点：
// 1. boa 的 GC 对象不是 Send 的，因此所有插件 Context 都驻留在一条专用线程上，
//    外部通过 channel + oneshot 与之通信（worker 模式，彻底回避线程安全问题）。
// 2. 沙箱默认零权限：无文件系统、无进程、无任意网络。宿主只注入 nova.* 白名单 API：
//    - nova.tool(name, handler)     注册工具实现（handler 同步或返回 Promise 均可）
//    - nova.getSetting / setSetting / getSettings  读写插件设置快照
//    - nova.http.get / postJson     受 net: 权限约束的 HTTP 访问
//    - nova.usage.getTotal / getToday / getRecent  宿主用量只读聚合（不含会话明细）
//    - nova.session.getInfo / listTools            当前会话信息与工具清单
//    - nova.host.getTheme()                        宿主 UI 主题
//    - nova.log(...)                写宿主日志
// 3. nova.http / nova.usage / nova.session / nova.host 均通过 channel 轮询桥接到
//    tokio 异步运行时（worker 线程阻塞等待是安全的）。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{mpsc, OnceLock};
use std::time::Duration;

use boa_engine::{
    js_string, Context, JsArgs, JsError, JsResult, JsValue, NativeFunction, Source,
    builtins::promise::PromiseState,
    object::JsObject,
    object::builtins::JsPromise,
    property::Attribute,
};
use serde_json::Value as Json;
use tauri::AppHandle;
use tokio::sync::oneshot;
use tracing::{info, warn};

use super::manifest::PluginManifest;

const HTTP_TIMEOUT: Duration = Duration::from_secs(30);
/// 宿主数据查询（SQLite / 设置读取）超时：远小于 HTTP 超时，防卡工具调用。
const HOST_QUERY_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) enum SandboxCommand {
    Load {
        plugin_id: String,
        code: String,
        settings: Json,
        net_patterns: Vec<String>,
        app: AppHandle,
        reply: oneshot::Sender<Result<Vec<String>, String>>,
    },
    Call {
        plugin_id: String,
        tool: String,
        args: Json,
        /// 当前工具调用所属会话（宿主数据桥 session.* 上下文）。
        conversation_id: Option<String>,
        reply: oneshot::Sender<Result<Json, String>>,
    },
    UpdateSettings {
        plugin_id: String,
        settings: Json,
    },
    Unload {
        plugin_id: String,
    },
}

/// 单个插件在 worker 线程内的运行时槽位。
struct PluginSlot {
    context: Context,
    tools: Rc<RefCell<HashMap<String, JsObject>>>,
    settings: Rc<RefCell<Json>>,
    /// 当前工具调用所属会话 id（每次 Call 前更新，nova.session.* 读取）。
    conversation: Rc<RefCell<Option<String>>>,
}

pub(crate) struct PluginSandbox {
    tx: mpsc::Sender<SandboxCommand>,
}

static SANDBOX: OnceLock<PluginSandbox> = OnceLock::new();

/// 获取全局沙箱单例（首次调用时启动 worker 线程）。
pub(crate) fn sandbox() -> &'static PluginSandbox {
    SANDBOX.get_or_init(start_worker)
}

fn start_worker() -> PluginSandbox {
    let (tx, rx) = mpsc::channel::<SandboxCommand>();
    std::thread::Builder::new()
        .name("nova-plugin-sandbox".to_string())
        .spawn(move || worker_loop(rx))
        .expect("failed to spawn plugin sandbox worker thread");
    PluginSandbox { tx }
}

fn worker_loop(rx: mpsc::Receiver<SandboxCommand>) {
    let mut slots: HashMap<String, PluginSlot> = HashMap::new();
    while let Ok(command) = rx.recv() {
        match command {
            SandboxCommand::Load {
                plugin_id,
                code,
                settings,
                net_patterns,
                app,
                reply,
            } => {
                let result =
                    load_plugin(&mut slots, &plugin_id, &code, settings, net_patterns, app);
                let _ = reply.send(result);
            }
            SandboxCommand::Call {
                plugin_id,
                tool,
                args,
                conversation_id,
                reply,
            } => {
                let result = call_tool(&mut slots, &plugin_id, &tool, args, conversation_id);
                let _ = reply.send(result);
            }
            SandboxCommand::UpdateSettings { plugin_id, settings } => {
                if let Some(slot) = slots.get(&plugin_id) {
                    *slot.settings.borrow_mut() = settings;
                }
            }
            SandboxCommand::Unload { plugin_id } => {
                if slots.remove(&plugin_id).is_some() {
                    info!(plugin = %plugin_id, "plugin sandbox slot unloaded");
                }
            }
        }
    }
}

fn load_plugin(
    slots: &mut HashMap<String, PluginSlot>,
    plugin_id: &str,
    code: &str,
    settings: Json,
    net_patterns: Vec<String>,
    app: AppHandle,
) -> Result<Vec<String>, String> {
    // 幂等加载：槽位已存在时仅刷新设置快照并返回已注册工具
    //（重载需先 Unload，由停用/启用流程触发）。
    if let Some(slot) = slots.get(plugin_id) {
        *slot.settings.borrow_mut() = settings;
        return Ok(slot.tools.borrow().keys().cloned().collect());
    }

    let mut context = Context::default();
    let tools: Rc<RefCell<HashMap<String, JsObject>>> = Rc::new(RefCell::new(HashMap::new()));
    let settings_rc: Rc<RefCell<Json>> = Rc::new(RefCell::new(settings));
    let conversation_rc: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

    build_nova_api(
        &mut context,
        tools.clone(),
        settings_rc.clone(),
        net_patterns,
        app,
        conversation_rc.clone(),
    )?;

    context
        .eval(Source::from_bytes(code))
        .map_err(|error| format!("plugin '{}' main.js failed: {}", plugin_id, error))?;

    let registered = tools.borrow().keys().cloned().collect::<Vec<_>>();
    // 槽位替换即卸载旧 Context（boa_gc 自动回收全部 JS 对象）。
    slots.insert(
        plugin_id.to_string(),
        PluginSlot {
            context,
            tools,
            settings: settings_rc,
            conversation: conversation_rc,
        },
    );
    info!(plugin = %plugin_id, tools = registered.len(), "plugin sandbox slot loaded");
    Ok(registered)
}

fn call_tool(
    slots: &mut HashMap<String, PluginSlot>,
    plugin_id: &str,
    tool: &str,
    args: Json,
    conversation_id: Option<String>,
) -> Result<Json, String> {
    let slot = slots
        .get_mut(plugin_id)
        .ok_or_else(|| format!("plugin '{}' is not loaded in sandbox", plugin_id))?;

    // 更新宿主数据桥的会话上下文（nova.session.getInfo 等读取）。
    *slot.conversation.borrow_mut() = conversation_id;

    let handler = slot
        .tools
        .borrow()
        .get(tool)
        .cloned()
        .ok_or_else(|| format!("plugin '{}' has no tool handler '{}'", plugin_id, tool))?;

    let arg_value = JsValue::from_json(&args, &mut slot.context)
        .map_err(|error| format!("failed to convert tool args for '{}': {}", tool, error))?;

    let raw = handler
        .call(&JsValue::undefined(), &[arg_value], &mut slot.context)
        .map_err(|error| format!("tool '{}' raised: {}", tool, error))?;

    let settled = settle_value(raw, &mut slot.context)
        .map_err(|error| format!("tool '{}' failed: {}", tool, error))?;

    to_json_value(&settled, &mut slot.context)
        .map_err(|error| format!("tool '{}' returned unserializable value: {}", tool, error))
}

/// handler 返回 Promise 时驱动 job 队列至其 settle，否则原样返回。
fn settle_value(value: JsValue, context: &mut Context) -> Result<JsValue, String> {
    let Some(object) = value.as_object() else {
        return Ok(value);
    };
    let Ok(promise) = JsPromise::from_object(object) else {
        return Ok(value);
    };
    let _ = context.run_jobs();
    match promise.state() {
        PromiseState::Fulfilled(v) => Ok(v),
        PromiseState::Rejected(err) => Err(display_js_value(&err, context)),
        PromiseState::Pending => Err("plugin handler promise did not settle".to_string()),
    }
}

/// boa 0.21 的 to_json 返回 Option（undefined → None），这里归一化为 Null。
fn to_json_value(value: &JsValue, context: &mut Context) -> JsResult<Json> {
    value.to_json(context).map(|opt| opt.unwrap_or(Json::Null))
}

fn display_js_value(value: &JsValue, context: &mut Context) -> String {
    if let Some(text) = value.as_string() {
        return text.to_std_string_escaped();
    }
    match to_json_value(value, context) {
        Ok(json) => json.to_string(),
        Err(_) => "[unserializable plugin error]".to_string(),
    }
}

fn js_type_error(message: &str) -> JsError {
    JsError::from_opaque(js_string!(message).into())
}

/// NativeFunction::from_closure 的安全封装。
///
/// SAFETY 契约：闭包不得捕获未 root 的 GC 对象。本沙箱内闭包只捕获
/// Rc<RefCell<…>>（HashMap 中的 JsObject 均来自 boa_gc 的 root-on-clone 克隆，
/// 在 Rust 侧持有期间始终是 GC 根），满足契约。
fn native_closure<F>(closure: F) -> NativeFunction
where
    F: Fn(&JsValue, &[JsValue], &mut Context) -> JsResult<JsValue> + 'static,
{
    unsafe { NativeFunction::from_closure(closure) }
}

/// 组装 nova.* 白名单 API 并挂到全局对象。
fn build_nova_api(
    context: &mut Context,
    tools: Rc<RefCell<HashMap<String, JsObject>>>,
    settings: Rc<RefCell<Json>>,
    net_patterns: Vec<String>,
    app: AppHandle,
    conversation: Rc<RefCell<Option<String>>>,
) -> Result<(), String> {
    let nova = JsObject::with_object_proto(context.intrinsics());

    // --- nova.tool(name, handler) ---
    let tools_for_register = tools.clone();
    let register_tool = native_closure(move |_this, args, context| {
        let name = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        if name.is_empty() {
            return Err(js_type_error("nova.tool: tool name must be a non-empty string"));
        }
        let handler_object = args
            .get_or_undefined(1)
            .as_object()
            .ok_or_else(|| js_type_error("nova.tool: handler must be a function"))?;
        if !handler_object.is_callable() {
            return Err(js_type_error("nova.tool: handler must be a function"));
        }
        tools_for_register.borrow_mut().insert(name, handler_object);
        Ok(JsValue::undefined())
    })
    .to_js_function(context.realm());
    nova.set(js_string!("tool"), register_tool, false, context)
        .map_err(|e| format!("failed to inject nova.tool: {}", e))?;

    // --- nova.log(...) ---
    let plugin_log = native_closure(move |_this, args, context| {
        let mut parts = Vec::with_capacity(args.len());
        for arg in args {
            parts.push(display_js_value(arg, context));
        }
        info!(target: "nova_plugin", "[plugin] {}", parts.join(" "));
        Ok(JsValue::undefined())
    })
    .to_js_function(context.realm());
    nova.set(js_string!("log"), plugin_log, false, context)
        .map_err(|e| format!("failed to inject nova.log: {}", e))?;

    // --- nova.getSettings() ---
    let settings_for_get = settings.clone();
    let get_settings = native_closure(move |_this, _args, context| {
        let snapshot = settings_for_get.borrow().clone();
        JsValue::from_json(&snapshot, context)
    })
    .to_js_function(context.realm());
    nova.set(js_string!("getSettings"), get_settings, false, context)
        .map_err(|e| format!("failed to inject nova.getSettings: {}", e))?;

    // --- nova.getSetting(key, default?) ---
    let settings_for_get_one = settings.clone();
    let get_setting = native_closure(move |_this, args, context| {
        let key = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        let snapshot = settings_for_get_one.borrow().clone();
        match snapshot.get(&key) {
            Some(value) => JsValue::from_json(value, context),
            None => Ok(args.get_or_undefined(1).clone()),
        }
    })
    .to_js_function(context.realm());
    nova.set(js_string!("getSetting"), get_setting, false, context)
        .map_err(|e| format!("failed to inject nova.getSetting: {}", e))?;

    // --- nova.setSetting(key, value) ---
    // 注意：沙箱内 setSetting 只更新内存快照（工具运行时可见），持久化由设置页桥完成。
    let settings_for_set = settings.clone();
    let set_setting = native_closure(move |_this, args, context| {
        let key = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        let value = args.get_or_undefined(1).clone();
        let json = to_json_value(&value, context)?;
        let mut snapshot = settings_for_set.borrow_mut();
        if !snapshot.is_object() {
            *snapshot = serde_json::json!({});
        }
        if let Some(map) = snapshot.as_object_mut() {
            map.insert(key, json);
        }
        Ok(JsValue::undefined())
    })
    .to_js_function(context.realm());
    nova.set(js_string!("setSetting"), set_setting, false, context)
        .map_err(|e| format!("failed to inject nova.setSetting: {}", e))?;

    // --- nova.http.{get,postJson} ---
    let http = JsObject::with_object_proto(context.intrinsics());
    let matchers = compile_net_matchers(net_patterns);
    let matchers_for_post = matchers.clone();

    let http_get = native_closure(move |_this, args, context| {
        let url = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        ensure_net_allowed(&matchers, &url)?;
        let body = http_request_blocking("GET", &url, None)?;
        Ok(JsValue::from(js_string!(body)))
    })
    .to_js_function(context.realm());
    http.set(js_string!("get"), http_get, false, context)
        .map_err(|e| format!("failed to inject nova.http.get: {}", e))?;

    let http_post = native_closure(move |_this, args, context| {
        let url = args
            .get_or_undefined(0)
            .to_string(context)?
            .to_std_string_escaped();
        let body = to_json_value(args.get_or_undefined(1), context)?.to_string();
        ensure_net_allowed(&matchers_for_post, &url)?;
        let body = http_request_blocking("POST", &url, Some(body))?;
        Ok(JsValue::from(js_string!(body)))
    })
    .to_js_function(context.realm());
    http.set(js_string!("postJson"), http_post, false, context)
        .map_err(|e| format!("failed to inject nova.http.postJson: {}", e))?;

    nova.set(js_string!("http"), http, false, context)
        .map_err(|e| format!("failed to inject nova.http: {}", e))?;

    // --- nova.usage.getTotal / getToday / getRecent（宿主用量只读聚合） ---
    let usage = JsObject::with_object_proto(context.intrinsics());

    let app_for_total = app.clone();
    let conv_for_total = conversation.clone();
    let usage_get_total = native_closure(move |_this, _args, context| {
        host_query_blocking(
            context,
            &app_for_total,
            conv_for_total.borrow().clone(),
            "usage.getTotal",
            None,
        )
    })
    .to_js_function(context.realm());
    usage
        .set(js_string!("getTotal"), usage_get_total, false, context)
        .map_err(|e| format!("failed to inject nova.usage.getTotal: {}", e))?;

    let app_for_today = app.clone();
    let conv_for_today = conversation.clone();
    let usage_get_today = native_closure(move |_this, _args, context| {
        host_query_blocking(
            context,
            &app_for_today,
            conv_for_today.borrow().clone(),
            "usage.getToday",
            None,
        )
    })
    .to_js_function(context.realm());
    usage
        .set(js_string!("getToday"), usage_get_today, false, context)
        .map_err(|e| format!("failed to inject nova.usage.getToday: {}", e))?;

    let app_for_recent = app.clone();
    let conv_for_recent = conversation.clone();
    let usage_get_recent = native_closure(move |_this, args, context| {
        let n = to_json_value(args.get_or_undefined(0), context)?;
        host_query_blocking(
            context,
            &app_for_recent,
            conv_for_recent.borrow().clone(),
            "usage.getRecent",
            Some(n),
        )
    })
    .to_js_function(context.realm());
    usage
        .set(js_string!("getRecent"), usage_get_recent, false, context)
        .map_err(|e| format!("failed to inject nova.usage.getRecent: {}", e))?;

    nova.set(js_string!("usage"), usage, false, context)
        .map_err(|e| format!("failed to inject nova.usage: {}", e))?;

    // --- nova.session.getInfo / listTools（当前会话信息与工具清单） ---
    let session = JsObject::with_object_proto(context.intrinsics());

    let app_for_info = app.clone();
    let conv_for_info = conversation.clone();
    let session_get_info = native_closure(move |_this, _args, context| {
        host_query_blocking(
            context,
            &app_for_info,
            conv_for_info.borrow().clone(),
            "session.getInfo",
            None,
        )
    })
    .to_js_function(context.realm());
    session
        .set(js_string!("getInfo"), session_get_info, false, context)
        .map_err(|e| format!("failed to inject nova.session.getInfo: {}", e))?;

    let app_for_tools = app.clone();
    let conv_for_tools = conversation.clone();
    let session_list_tools = native_closure(move |_this, _args, context| {
        host_query_blocking(
            context,
            &app_for_tools,
            conv_for_tools.borrow().clone(),
            "session.listTools",
            None,
        )
    })
    .to_js_function(context.realm());
    session
        .set(js_string!("listTools"), session_list_tools, false, context)
        .map_err(|e| format!("failed to inject nova.session.listTools: {}", e))?;

    nova.set(js_string!("session"), session, false, context)
        .map_err(|e| format!("failed to inject nova.session: {}", e))?;

    // --- nova.host.getTheme()（宿主 UI 主题） ---
    let host = JsObject::with_object_proto(context.intrinsics());
    let app_for_theme = app;
    let host_get_theme = native_closure(move |_this, _args, context| {
        host_query_blocking(context, &app_for_theme, None, "host.getTheme", None)
    })
    .to_js_function(context.realm());
    host.set(js_string!("getTheme"), host_get_theme, false, context)
        .map_err(|e| format!("failed to inject nova.host.getTheme: {}", e))?;

    nova.set(js_string!("host"), host, false, context)
        .map_err(|e| format!("failed to inject nova.host: {}", e))?;

    context
        .register_global_property(js_string!("nova"), nova, Attribute::all())
        .map_err(|e| format!("failed to register global nova object: {}", e))?;
    Ok(())
}

// ---------------- 宿主数据查询桥 ----------------

/// 在 worker 阻塞线程上执行宿主异步查询：抛给 tokio 运行时，本地 channel 轮询结果。
/// 与 http_request_blocking 同一模式（worker 线程阻塞等待是安全的）。
fn host_query_blocking(
    context: &mut Context,
    app: &AppHandle,
    conversation_id: Option<String>,
    kind: &str,
    arg: Option<Json>,
) -> JsResult<JsValue> {
    let (tx, rx) = mpsc::channel::<Result<Json, String>>();
    let app = app.clone();
    let kind = kind.to_string();
    let kind_for_error = kind.clone();
    tauri::async_runtime::spawn(async move {
        let result = run_host_query(&app, conversation_id.as_deref(), &kind, arg).await;
        let _ = tx.send(result);
    });
    match rx.recv_timeout(HOST_QUERY_TIMEOUT) {
        Ok(Ok(json)) => JsValue::from_json(&json, context),
        Ok(Err(error)) => Err(js_type_error(&format!("nova.{}: {}", kind_for_error, error))),
        Err(_) => Err(js_type_error(&format!(
            "nova.{}: query timed out after {}s",
            kind_for_error,
            HOST_QUERY_TIMEOUT.as_secs()
        ))),
    }
}

/// 宿主数据查询分派（tokio 侧执行）。
async fn run_host_query(
    app: &AppHandle,
    conversation_id: Option<&str>,
    kind: &str,
    arg: Option<Json>,
) -> Result<Json, String> {
    use crate::llm::services::token_usage_log;
    match kind {
        "usage.getTotal" => token_usage_log::get_global_usage_totals(app).await,
        "usage.getToday" => token_usage_log::get_today_usage_totals(app).await,
        "usage.getRecent" => {
            let n = arg.as_ref().and_then(|v| v.as_i64()).unwrap_or(10);
            token_usage_log::get_recent_usage_records(app, n).await
        }
        "session.getInfo" => host_session_info(app, conversation_id).await,
        "session.listTools" => host_session_list_tools(app).await,
        "host.getTheme" => host_theme(app).await,
        _ => Err(format!("unknown host query '{}'", kind)),
    }
}

/// 会话信息：模型 / provider / 工作区路径 / 标题 / 开始时间。
async fn host_session_info(
    app: &AppHandle,
    conversation_id: Option<&str>,
) -> Result<Json, String> {
    let settings = crate::command::settings::get_settings(app.clone())?;
    let profile = settings.provider_profiles.get(&settings.provider);
    let model = profile.map(|p| p.model.clone()).unwrap_or_default();
    let workspace = crate::command::workspace::workspace_root_for_conversation(
        app,
        conversation_id,
    )
    .map(|path| path.display().to_string())
    .unwrap_or_default();

    let mut info = serde_json::json!({
        "conversationId": conversation_id,
        "model": model,
        "provider": settings.provider,
        "workspacePath": workspace,
    });

    // 会话元数据（标题/开始时间）：仅在真实会话内调用时补充。
    if let Some(cid) = conversation_id.map(str::trim).filter(|s| !s.is_empty()) {
        if let Ok(brief) =
            crate::llm::services::token_usage_log::get_conversation_brief(app, cid).await
        {
            if let (Some(title), Some(started)) = (
                brief.get("title").cloned(),
                brief.get("startedAt").cloned(),
            ) {
                if let Some(map) = info.as_object_mut() {
                    map.insert("title".to_string(), title);
                    map.insert("startedAt".to_string(), started);
                }
            }
        }
    }
    Ok(info)
}

/// 当前可用工具清单（builtin / plugin / mcp 三来源）。
async fn host_session_list_tools(app: &AppHandle) -> Result<Json, String> {
    let mut tools: Vec<Json> = Vec::new();
    for name in crate::llm::tools::builtin_tool_names() {
        tools.push(serde_json::json!({ "name": name, "source": "builtin" }));
    }
    for tool in crate::llm::services::plugins::plugin_tools(app) {
        tools.push(serde_json::json!({ "name": tool.name, "source": "plugin" }));
    }
    for tool in crate::llm::services::mcp_tools::collect_mcp_tools(app).await {
        tools.push(serde_json::json!({ "name": tool.name, "source": "mcp" }));
    }
    Ok(Json::Array(tools))
}

/// 宿主 UI 主题：system 时取主窗口实际值。
async fn host_theme(app: &AppHandle) -> Result<Json, String> {
    let settings = crate::command::settings::get_settings(app.clone())?;
    let theme = match settings.ui_theme.as_str() {
        "light" => "light".to_string(),
        "dark" => "dark".to_string(),
        _ => {
            use tauri::Manager;
            let actual = app.get_webview_window("main").and_then(|w| w.theme().ok());
            match actual {
                Some(tauri::Theme::Dark) => "dark".to_string(),
                _ => "light".to_string(),
            }
        }
    };
    Ok(serde_json::json!(theme))
}

fn compile_net_matchers(patterns: Vec<String>) -> Vec<globset::GlobMatcher> {
    patterns
        .into_iter()
        .filter_map(|pattern| {
            match globset::GlobBuilder::new(&pattern)
                .literal_separator(false)
                .build()
            {
                Ok(glob) => Some(glob.compile_matcher()),
                Err(error) => {
                    warn!(pattern = %pattern, error = %error, "invalid net permission glob");
                    None
                }
            }
        })
        .collect()
}

fn ensure_net_allowed(matchers: &[globset::GlobMatcher], url: &str) -> JsResult<()> {
    if url.starts_with("http://localhost") || url.starts_with("http://127.0.0.1") {
        return Err(js_type_error(
            "nova.http: localhost requests are blocked for security",
        ));
    }
    if matchers.iter().any(|matcher| matcher.is_match(url)) {
        Ok(())
    } else {
        Err(js_type_error(&format!(
            "nova.http: url '{}' is not covered by any net: permission in plugin.json",
            url
        )))
    }
}

/// 在 worker 阻塞线程上发起 HTTP 请求：把实际 IO 抛给 tokio 运行时，本地 channel 轮询结果。
fn http_request_blocking(method: &str, url: &str, body: Option<String>) -> JsResult<String> {
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let method_owned = method.to_string();
    let url_owned = url.to_string();
    tauri::async_runtime::spawn(async move {
        let result = perform_http(&method_owned, &url_owned, body).await;
        let _ = tx.send(result);
    });
    match rx.recv_timeout(HTTP_TIMEOUT) {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(error)) => Err(js_type_error(&format!("nova.http {}: {}", method, error))),
        Err(_) => Err(js_type_error(&format!(
            "nova.http {}: request timed out after {}s",
            method,
            HTTP_TIMEOUT.as_secs()
        ))),
    }
}

async fn perform_http(method: &str, url: &str, body: Option<String>) -> Result<String, String> {
    let client = reqwest::Client::builder()
        .timeout(HTTP_TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let request = match method {
        "POST" => {
            let mut req = client.post(url);
            if let Some(payload) = body {
                req = req
                    .header(reqwest::header::CONTENT_TYPE, "application/json")
                    .body(payload);
            }
            req
        }
        _ => client.get(url),
    };
    let response = request.send().await.map_err(|e| e.to_string())?;
    let status = response.status();
    let text = response.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("HTTP {} returned status {}", method, status));
    }
    Ok(text)
}

impl PluginSandbox {
    pub(crate) async fn load(
        &self,
        plugin_id: &str,
        code: String,
        settings: Json,
        manifest: &PluginManifest,
        app: AppHandle,
    ) -> Result<Vec<String>, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(SandboxCommand::Load {
                plugin_id: plugin_id.to_string(),
                code,
                settings,
                net_patterns: manifest.net_permission_patterns(),
                app,
                reply: reply_tx,
            })
            .map_err(|_| "plugin sandbox worker is gone".to_string())?;
        reply_rx
            .await
            .map_err(|_| "plugin sandbox worker dropped the load reply".to_string())?
    }

    pub(crate) async fn call(
        &self,
        plugin_id: &str,
        tool: &str,
        args: Json,
        conversation_id: Option<String>,
    ) -> Result<Json, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(SandboxCommand::Call {
                plugin_id: plugin_id.to_string(),
                tool: tool.to_string(),
                args,
                conversation_id,
                reply: reply_tx,
            })
            .map_err(|_| "plugin sandbox worker is gone".to_string())?;
        reply_rx
            .await
            .map_err(|_| "plugin sandbox worker dropped the call reply".to_string())?
    }

    pub(crate) fn update_settings(&self, plugin_id: &str, settings: Json) {
        let _ = self.tx.send(SandboxCommand::UpdateSettings {
            plugin_id: plugin_id.to_string(),
            settings,
        });
    }

    pub(crate) fn unload(&self, plugin_id: &str) {
        let _ = self.tx.send(SandboxCommand::Unload {
            plugin_id: plugin_id.to_string(),
        });
    }
}
