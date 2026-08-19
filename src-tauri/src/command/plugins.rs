// 插件管理 IPC 命令 + nova-plugin:// URI 协议处理器。
//
// URI 协议只服务两类资源：
//   /__bridge.js            宿主内置桥脚本（插件 HTML 通过 <script src="/__bridge.js"> 引入）
//   /<plugin-id>/<file...>  插件目录内 ui/ 等静态文件（仅限已启用插件 + 白名单扩展名）
// 安全约束：路径穿越拒绝、canonicalize 前缀校验、扩展名白名单、禁用插件 404。

use serde_json::Value as Json;
use tauri::{AppHandle, UriSchemeContext, Wry};
use tauri_plugin_opener::OpenerExt;

use crate::llm::services::plugins::{self, list_plugins as scan_plugins, PluginInfo};

#[tauri::command]
pub async fn list_plugins(app: AppHandle) -> Result<Vec<PluginInfo>, String> {
    // 首次使用时自动生成示例插件，让用户开箱即可看到完整链路效果。
    ensure_sample_plugin(&app)?;
    Ok(scan_plugins(&app))
}

#[tauri::command]
pub async fn set_plugin_enabled(
    app: AppHandle,
    plugin_id: String,
    enabled: bool,
) -> Result<(), String> {
    plugins::set_plugin_enabled(&app, &plugin_id, enabled)
}

#[tauri::command]
pub async fn get_plugin_settings(app: AppHandle, plugin_id: String) -> Result<Json, String> {
    let valid = scan_plugins(&app)
        .into_iter()
        .any(|info| info.id == plugin_id && info.error.is_none());
    if !valid {
        return Err(format!("unknown plugin '{}'", plugin_id));
    }
    Ok(plugins::read_plugin_settings(&app, &plugin_id))
}

#[tauri::command]
pub async fn set_plugin_settings(
    app: AppHandle,
    plugin_id: String,
    settings: Json,
) -> Result<(), String> {
    plugins::write_plugin_settings(&app, &plugin_id, &settings)
}

#[tauri::command]
pub async fn call_plugin_tool(
    app: AppHandle,
    plugin_id: String,
    tool: String,
    args: Json,
) -> Result<Json, String> {
    plugins::call_plugin_tool_direct(&app, &plugin_id, &tool, args).await
}

#[tauri::command]
pub fn open_plugins_dir(app: AppHandle) -> Result<String, String> {
    let root = plugins::plugins_root(&app);
    std::fs::create_dir_all(&root).map_err(|e| format!("failed to create plugins dir: {}", e))?;
    let path = root.to_string_lossy().to_string();
    // 打开目录所在位置，交给系统资源管理器。
    let _ = app.opener().open_path(&path, None::<&str>);
    Ok(path)
}

/// 处理 nova-plugin:// 协议请求。
pub fn handle_plugin_uri(
    context: UriSchemeContext<'_, Wry>,
    request: tauri::http::Request<Vec<u8>>,
) -> tauri::http::Response<Vec<u8>> {
    let app = context.app_handle();
    let uri = request.uri().to_string();
    // Windows(WebView2) 侧请求形如 http://nova-plugin.localhost/<path>；
    // macOS/Linux 侧形如 nova-plugin://localhost/<path>。统一切出 localhost 之后的路径。
    let raw_path = uri
        .split_once("localhost")
        .map(|(_, rest)| rest.trim_start_matches('/'))
        .unwrap_or("");
    let decoded = urlencoding::decode(raw_path)
        .map(|cow| cow.into_owned())
        .unwrap_or_else(|_| raw_path.to_string());

    let response = |status: u16, content_type: &str, body: Vec<u8>| {
        tauri::http::Response::builder()
            .status(status)
            .header("Content-Type", content_type)
            .header("Cache-Control", "no-store")
            .body(body)
            .unwrap_or_else(|_| tauri::http::Response::new(Vec::new()))
    };

    if decoded == "__bridge.js" {
        return response(200, "application/javascript; charset=utf-8", BRIDGE_JS.as_bytes().to_vec());
    }

    if decoded.is_empty() {
        return response(404, "text/plain; charset=utf-8", b"not found".to_vec());
    }

    // 解析 /<plugin-id>/<file...>
    let (plugin_id, file_path) = match decoded.split_once('/') {
        Some((id, rest)) if !rest.is_empty() => (id.to_string(), rest.to_string()),
        _ => return response(404, "text/plain; charset=utf-8", b"not found".to_vec()),
    };

    let valid_id = !plugin_id.is_empty()
        && plugin_id.len() <= 64
        && plugin_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid_id {
        return response(404, "text/plain; charset=utf-8", b"not found".to_vec());
    }

    // 路径安全：拒绝穿越与反斜杠，限制扩展名白名单。
    if file_path.contains("..") || file_path.contains('\\') {
        return response(403, "text/plain; charset=utf-8", b"forbidden".to_vec());
    }
    let extension = file_path
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase();
    let content_type = match extension.as_str() {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "application/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" => "application/json; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "ico" => "image/x-icon",
        "txt" | "md" => "text/plain; charset=utf-8",
        _ => return response(403, "text/plain; charset=utf-8", b"forbidden".to_vec()),
    };

    // 仅已启用且解析成功的插件可供给文件。
    let enabled = scan_plugins(app)
        .into_iter()
        .any(|info| info.id == plugin_id && info.enabled && info.error.is_none());
    if !enabled {
        return response(404, "text/plain; charset=utf-8", b"not found".to_vec());
    }

    let root = plugins::plugins_root(app);
    let full_path = root.join(&plugin_id).join(&file_path);
    let canonical = match std::fs::canonicalize(&full_path) {
        Ok(path) => path,
        Err(_) => return response(404, "text/plain; charset=utf-8", b"not found".to_vec()),
    };
    let canonical_root = match std::fs::canonicalize(root.join(&plugin_id)) {
        Ok(path) => path,
        Err(_) => return response(404, "text/plain; charset=utf-8", b"not found".to_vec()),
    };
    if !canonical.starts_with(&canonical_root) {
        return response(403, "text/plain; charset=utf-8", b"forbidden".to_vec());
    }

    match std::fs::read(&canonical) {
        Ok(bytes) => response(200, content_type, bytes),
        Err(_) => response(404, "text/plain; charset=utf-8", b"not found".to_vec()),
    }
}

/// 首次运行时落盘示例插件（插件根目录不存在才写入，幂等）。
fn ensure_sample_plugin(app: &AppHandle) -> Result<(), String> {
    let root = plugins::plugins_root(app);
    if root.exists() {
        return Ok(());
    }
    let sample_dir = root.join("text-tools");
    std::fs::create_dir_all(sample_dir.join("ui"))
        .map_err(|e| format!("failed to create sample plugin dir: {}", e))?;
    std::fs::write(sample_dir.join("plugin.json"), SAMPLE_PLUGIN_JSON)
        .map_err(|e| format!("failed to write sample plugin.json: {}", e))?;
    std::fs::write(sample_dir.join("main.js"), SAMPLE_MAIN_JS)
        .map_err(|e| format!("failed to write sample main.js: {}", e))?;
    std::fs::write(sample_dir.join("ui").join("settings.html"), SAMPLE_SETTINGS_HTML)
        .map_err(|e| format!("failed to write sample settings.html: {}", e))?;
    Ok(())
}

const SAMPLE_PLUGIN_JSON: &str = r#"{
  "id": "text-tools",
  "name": "文本工坊",
  "version": "0.1.0",
  "description": "示例插件：为 AI 提供字数统计与文本转换两个工具，并附带一个设置页演示插件界面能力。停用后工具与设置页同步消失。",
  "author": "Nova",
  "permissions": [],
  "contributes": {
    "tools": [
      {
        "name": "word_count",
        "description": "统计文本的字符数（含/不含空白）、单词数与行数。",
        "parameters": {
          "type": "object",
          "properties": {
            "text": { "type": "string", "description": "要统计的文本" }
          },
          "required": ["text"]
        }
      },
      {
        "name": "text_case",
        "description": "转换文本：mode=upper 转大写，lower 转小写，reverse 反转字符顺序。不传 mode 时使用设置页配置的默认模式。",
        "parameters": {
          "type": "object",
          "properties": {
            "text": { "type": "string", "description": "要转换的文本" },
            "mode": { "type": "string", "enum": ["upper", "lower", "reverse"] }
          },
          "required": ["text"]
        }
      }
    ],
    "settingsTab": {
      "title": "文本工坊",
      "icon": "T",
      "view": "ui/settings.html"
    }
  }
}
"#;

const SAMPLE_MAIN_JS: &str = r#"// 文本工坊示例插件：演示 nova.tool 注册与 nova.getSetting 读取插件设置。
// 沙箱内无 fs/进程/任意网络，只有 nova.* 白名单 API。

nova.tool("word_count", function (args) {
  const text = String(args.text ?? "");
  const chars = Array.from(text).length;
  const withoutWhitespace = Array.from(text.replace(/\s/g, "")).length;
  const words = text.trim() ? text.trim().split(/\s+/).length : 0;
  const lines = text ? text.split(/\r\n|\r|\n/).length : 0;
  return { chars, withoutWhitespace, words, lines };
});

nova.tool("text_case", function (args) {
  const text = String(args.text ?? "");
  const mode = String(args.mode || nova.getSetting("defaultMode") || "upper");
  switch (mode) {
    case "upper":
      return { mode, result: text.toUpperCase() };
    case "lower":
      return { mode, result: text.toLowerCase() };
    case "reverse":
      return { mode, result: Array.from(text).reverse().join("") };
    default:
      return { mode, result: text, hint: "未知模式，已原样返回" };
  }
});

nova.log("text-tools 插件已加载");
"#;

const SAMPLE_SETTINGS_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>文本工坊</title>
<script src="/__bridge.js"></script>
<style>
  :root {
    --bg: #fcfcfc; --surface: #ffffff; --border: #e5e7eb;
    --text: #1a1a1a; --muted: #64748b; --brand: #2563eb; --brand-soft: #eff6ff;
  }
  :root[data-theme="dark"] {
    --bg: #1a1a1a; --surface: #242424; --border: #3a3a3a;
    --text: #ececec; --muted: #a3a3a3; --brand: #60a5fa; --brand-soft: #1e293b;
  }
  * { box-sizing: border-box; }
  body {
    margin: 0; padding: 20px; background: var(--bg); color: var(--text);
    font-family: -apple-system, "Segoe UI", "Microsoft YaHei", "PingFang SC", sans-serif;
    font-size: 14px; line-height: 1.6;
  }
  h2 { margin: 0 0 4px; font-size: 16px; }
  .desc { color: var(--muted); font-size: 12.5px; margin: 0 0 18px; }
  .card {
    background: var(--surface); border: 1px solid var(--border);
    border-radius: 12px; padding: 16px; margin-bottom: 14px;
  }
  .card h3 { margin: 0 0 10px; font-size: 13.5px; }
  label { display: block; font-size: 12.5px; color: var(--muted); margin-bottom: 6px; }
  select, textarea {
    width: 100%; padding: 8px 10px; border: 1px solid var(--border); border-radius: 8px;
    background: var(--surface); color: var(--text); font-size: 13.5px; outline: none;
  }
  textarea { min-height: 96px; resize: vertical; font-family: inherit; }
  select:focus, textarea:focus { border-color: var(--brand); }
  button {
    padding: 7px 16px; border: none; border-radius: 8px; background: var(--brand);
    color: #fff; font-size: 13px; cursor: pointer;
  }
  button:disabled { opacity: 0.6; cursor: default; }
  .row { display: flex; gap: 10px; align-items: center; }
  .result {
    margin-top: 10px; padding: 10px 12px; border-radius: 8px; background: var(--brand-soft);
    font-size: 12.5px; color: var(--text); white-space: pre-wrap; word-break: break-all;
  }
  .result.err { background: transparent; border: 1px solid #ef4444; color: #ef4444; }
  .ok-tip { color: #22c55e; font-size: 12px; margin-left: 8px; opacity: 0; transition: opacity .2s; }
  .ok-tip.show { opacity: 1; }
</style>
</head>
<body>
  <h2>✍️ 文本工坊</h2>
  <p class="desc">插件设置页演示：这里的一切运行在 iframe 沙箱中，通过 postMessage 桥与宿主通信。</p>

  <div class="card">
    <h3>默认转换模式</h3>
    <label>text_case 工具未显式传 mode 时使用的默认值</label>
    <div class="row">
      <select id="mode">
        <option value="upper">upper（转大写）</option>
        <option value="lower">lower（转小写）</option>
        <option value="reverse">reverse（反转）</option>
      </select>
      <button id="save">保存</button>
      <span class="ok-tip" id="tip">已保存</span>
    </div>
  </div>

  <div class="card">
    <h3>实时测试 word_count 工具</h3>
    <label>这里直接调用插件自己注册的 AI 工具，验证沙箱执行链路</label>
    <textarea id="input">Nova 插件系统示例：hello world 你好世界</textarea>
    <div class="row" style="margin-top:10px">
      <button id="run">运行工具</button>
    </div>
    <div class="result" id="result" hidden></div>
  </div>

<script>
  (async function () {
    await NovaPlugin.ready;
    const mode = document.getElementById('mode');
    const input = document.getElementById('input');
    const result = document.getElementById('result');
    const tip = document.getElementById('tip');

    const settings = await NovaPlugin.getSettings();
    if (settings.defaultMode) mode.value = settings.defaultMode;

    document.getElementById('save').addEventListener('click', async () => {
      await NovaPlugin.setSettings({ ...settings, defaultMode: mode.value });
      tip.classList.add('show');
      setTimeout(() => tip.classList.remove('show'), 1600);
    });

    document.getElementById('run').addEventListener('click', async () => {
      result.hidden = false;
      result.className = 'result';
      result.textContent = '执行中…';
      try {
        const output = await NovaPlugin.callTool('word_count', { text: input.value });
        result.textContent = JSON.stringify(output, null, 2);
      } catch (e) {
        result.className = 'result err';
        result.textContent = String(e && e.message ? e.message : e);
      }
    });
  })();
</script>
</body>
</html>
"#;

/// 宿主内置桥脚本：插件 UI 通过它访问受控的宿主能力。
/// 协议：nova:ready → nova:hello（握手+token）→ nova:getSettings/setSettings/callTool（带 token + seq 关联）。
const BRIDGE_JS: &str = r#"(function () {
  'use strict';
  var token = null;
  var plugin = null;
  var seq = 0;
  var pending = new Map();
  var themeListeners = [];
  var readyResolve;
  var readyPromise = new Promise(function (resolve) { readyResolve = resolve; });

  window.addEventListener('message', function (event) {
    var msg = event.data;
    if (!msg || typeof msg !== 'object') return;
    if (msg.channel === 'nova:hello') {
      token = msg.token;
      plugin = msg.plugin || null;
      document.documentElement.dataset.theme = msg.theme === 'dark' ? 'dark' : 'light';
      readyResolve({ settings: msg.settings, theme: msg.theme, plugin: plugin });
      return;
    }
    if (msg.channel === 'nova:theme') {
      document.documentElement.dataset.theme = msg.theme === 'dark' ? 'dark' : 'light';
      themeListeners.forEach(function (cb) { try { cb(msg.theme); } catch (e) {} });
      return;
    }
    var handler = pending.get(msg.seq);
    if (!handler) return;
    pending.delete(msg.seq);
    if (msg.channel === 'nova:error' || msg.ok === false) {
      handler.reject(new Error(msg.error || 'bridge error'));
    } else {
      handler.resolve(msg.result !== undefined ? msg.result : msg.value);
    }
  });

  function request(channel, payload, timeoutMs) {
    if (!token) return Promise.reject(new Error('NovaPlugin 尚未连接（请等待 NovaPlugin.ready）'));
    return new Promise(function (resolve, reject) {
      var id = ++seq;
      pending.set(id, { resolve: resolve, reject: reject });
      parent.postMessage(Object.assign({ channel: channel, token: token, seq: id }, payload), '*');
      setTimeout(function () {
        if (pending.has(id)) {
          pending.delete(id);
          reject(new Error('bridge 超时: ' + channel));
        }
      }, timeoutMs || 20000);
    });
  }

  window.NovaPlugin = {
    ready: readyPromise,
    getPlugin: function () { return plugin; },
    getSettings: function () { return request('nova:getSettings', {}); },
    setSettings: function (settings) { return request('nova:setSettings', { settings: settings }); },
    callTool: function (tool, args) { return request('nova:callTool', { tool: tool, args: args || {} }, 60000); },
    onTheme: function (cb) {
      themeListeners.push(cb);
      return document.documentElement.dataset.theme;
    },
    get theme() { return document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light'; },
  };

  // 桥就绪后立即发起握手。
  parent.postMessage({ channel: 'nova:ready' }, '*');
})();
"#;
