import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const rootDir = path.resolve(__dirname, '..');

// 由桌面壳（src-tauri）直接处理的命令，不经内核通道分发。
const DESKTOP_LOCAL = new Set([
  'get_app_data_dir',
  'browser_navigate_window',
  'browser_reload_window',
  'browser_eval_window_script',
  'browser_eval_window_script_result',
  'browser_call_devtools_protocol_method',
  'browser_automation_result',
  'register_browser_session',
  'unregister_browser_session',
  'update_browser_session_url',
  'load_browser_tab_state',
  'save_browser_tab_state',
  'clear_browser_tab_state',
  'user_terminal_start',
  'user_terminal_write',
  'user_terminal_resize',
  'user_terminal_stop',
]);

// 内核内部函数：由后台子系统直接调用，不作为 RPC 方法暴露。
const INTERNAL_API = new Set([
  'run_scheduler_loop',
  'warmup_runtime',
  'delete_all_session_files',
  'delete_all_session_files_all',
]);

function walk(dir, exts, out = []) {
  for (const entry of fs.readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      if (entry.name === 'node_modules' || entry.name.startsWith('.')) continue;
      walk(full, exts, out);
    } else if (exts.some((ext) => entry.name.endsWith(ext))) {
      out.push(full);
    }
  }
  return out;
}

function collect(names, re, text) {
  let match;
  while ((match = re.exec(text)) !== null) {
    names.add(match[1]);
  }
}

// 前端实际 invoke / ipcInvoke 的命令名
const frontend = new Set();
for (const file of walk(path.join(rootDir, 'src'), ['.ts', '.vue'])) {
  collect(frontend, /(?:ipcInvoke|invoke)(?:<[^>]*>)?\(\s*['"]([a-z0-9_]+)['"]/g, fs.readFileSync(file, 'utf8'));
}

// 内核 RPC 分发器支持的方法名
const dispatch = new Set();
collect(
  dispatch,
  /^\s*"([a-z0-9_]+)"\s*=>/gm,
  fs.readFileSync(path.join(rootDir, 'crates/nova-cli/src/serve/dispatch.rs'), 'utf8'),
);

// nova-core 应用服务层暴露的函数名
const api = new Set();
const apiDir = path.join(rootDir, 'crates/nova-core/src/api');
for (const name of fs.readdirSync(apiDir)) {
  if (!name.endsWith('.rs')) continue;
  collect(api, /pub\s+(?:async\s+)?fn\s+([a-z0-9_]+)/g, fs.readFileSync(path.join(apiDir, name), 'utf8'));
}

const frontendMissing = [...frontend]
  .filter((name) => !dispatch.has(name) && !DESKTOP_LOCAL.has(name))
  .sort();
const apiMissing = [...api]
  .filter((name) => !dispatch.has(name) && !INTERNAL_API.has(name))
  .sort();

// 校验 Rust `nova-protocol` 绑定导出覆盖率与零 `bigint` 漂移
const protocolTypes = new Set();
for (const file of walk(path.join(rootDir, 'crates/nova-protocol/src'), ['.rs'])) {
  collect(
    protocolTypes,
    /#\[ts\([^\]]*export_to\s*=\s*"protocol\.ts"[^\]]*\)\][\s\S]*?\bpub\s+(?:struct|enum)\s+([A-Za-z0-9_]+)/g,
    fs.readFileSync(file, 'utf8'),
  );
}
const protocolTs = fs.readFileSync(path.join(rootDir, 'src/lib/protocol.ts'), 'utf8');
const contractSyncRs = fs.readFileSync(
  path.join(rootDir, 'crates/nova-protocol/tests/contract_sync.rs'),
  'utf8',
);
const protocolMissingInTs = [...protocolTypes]
  .filter((name) => !new RegExp(`\\bexport\\s+(?:type|interface)\\s+${name}\\b`).test(protocolTs))
  .sort();
const protocolMissingInSyncTest = [...protocolTypes]
  .filter((name) => !new RegExp(`::${name}\\b`).test(contractSyncRs))
  .sort();
const hasBigintInProtocol = /\bbigint\b/.test(protocolTs);

console.log(
  `[contract] 前端 invoke=${frontend.size} dispatch=${dispatch.size} api=${api.size} ` +
    `(桌面本地=${DESKTOP_LOCAL.size}, 内核内部=${INTERNAL_API.size}), protocol=${protocolTypes.size}`,
);

let failed = false;
if (frontendMissing.length > 0) {
  failed = true;
  console.error('\n[contract] 前端调用了不存在的方法（会静默失败）:');
  for (const name of frontendMissing) console.error(`  - ${name}`);
}
if (apiMissing.length > 0) {
  failed = true;
  console.error('\n[contract] api 中的函数未在 dispatch 中暴露:');
  for (const name of apiMissing) console.error(`  - ${name}`);
}
if (protocolMissingInTs.length > 0) {
  failed = true;
  console.error('\n[contract] nova-protocol 类型未导出到 src/lib/protocol.ts（请运行 npm run gen:bindings）:');
  for (const name of protocolMissingInTs) console.error(`  - ${name}`);
}
if (protocolMissingInSyncTest.length > 0) {
  failed = true;
  console.error('\n[contract] nova-protocol 类型未注册到 crates/nova-protocol/tests/contract_sync.rs:');
  for (const name of protocolMissingInSyncTest) console.error(`  - ${name}`);
}
if (hasBigintInProtocol) {
  failed = true;
  console.error('\n[contract] src/lib/protocol.ts 出现禁止的 `bigint` 类型（请改为 number 注解）');
}

if (failed) process.exit(1);
console.log('[contract] 三方契约与 protocol 绑定一致');
