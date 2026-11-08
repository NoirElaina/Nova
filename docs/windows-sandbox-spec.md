# Windows 沙箱实现规格（对照 Codex `windows-sandbox-rs`）

> 参考源码：`D:\ref\codex\codex-rs\windows-sandbox-rs`（Apache-2.0）
> 关键文件：`src/token.rs`、`src/process.rs`、`src/acl.rs`、`src/cap.rs`、`src/spawn_prep.rs`
>
> 本文档记录已核实的实现细节，供 `crates/nova-core/src/agent/sandbox/windows.rs` 落地。

## 一、为什么不用 AppContainer

OpenAI 官方明确否决了三个方案：

| 方案 | 否决原因 |
|---|---|
| AppContainer | 文件读取默认拒绝，而 agent 要读仓库、工具链、Git 配置、依赖缓存。打洞打到最后边界形同虚设 |
| Windows Sandbox | 轻量 VM，需要 host/guest 桥接；**Windows Home 版不可用** |
| MIC 完整性标签 | 要把整个工作区标为 low integrity，等于把用户真实 checkout 变成 low-IL 写入目标，信任模型被永久改变 |

实际采用：**write-restricted token + 合成 SID + ACL**。

## 二、核心机制

`WRITE_RESTRICTED` 令牌的语义：**只在写操作时**额外检查"限制 SID 列表中是否至少有一个对该对象有写权限"。
读操作不受影响 → 全局可读，只有写被收窄。这正是 `workspace-write` 想要的形状。

## 三、实现步骤

### 1. 合成 SID（`cap.rs`）

```rust
// 形如域账户 SID，但不存在对应账户，只出现在 ACL 里
format!("S-1-5-21-{a}-{b}-{c}-{d}")   // a..d 为随机 u32
```

- 每个可写根目录一个专属 SID
- 持久化到 `data_dir/sandbox_caps.json`，按规范化路径索引（`S-1-5-21-...` 字符串）
- 已存在则复用，不重复生成

### 2. 受限令牌（`token.rs:create_token_with_caps_from`）

```rust
const WRITE_RESTRICTED: u32 = 0x08;
let flags = DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED;

// RestrictingSids 顺序（Codex 注释明确要求）：
// Capabilities..., ExtraRestricting..., Logon, Everyone
CreateRestrictedToken(
    base_token,            // OpenProcessToken(GetCurrentProcess())
    flags,
    0, null,               // SidsToDisable
    0, null,               // PrivilegesToDelete
    entries.len(),
    entries.as_mut_ptr(),  // [合成SID..., Logon SID, Everyone]
    &mut new_token,
);
```

创建后还要：

- `SetTokenInformation(TokenDefaultDacl, ...)` 用 Logon SID 重设默认 DACL，隔离同机其他进程
- `AdjustTokenPrivileges` 启用 `SeChangeNotifyPrivilege`（遍历目录必需，否则很多工具挂）

**关键：restricting SIDs 不包含用户 SID**，所以写在任何未授权路径都会被拒（包括自己的家目录）。

### 3. ACL 授权（`acl.rs`）

```rust
// 给工作区加允许 ACE（合成 SID，继承到子目录）
EXPLICIT_ACCESS_W {
    grfAccessPermissions: allow_mask,   // 写/执行/删除
    grfAccessMode: GRANT_ACCESS,
    grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    Trustee: TRUSTEE_W { TrusteeForm: TRUSTEE_IS_SID, ptstrName: sid as *mut u16, .. },
}
SetEntriesInAclW(1, &explicit, p_dacl, &mut p_new_dacl)
SetSecurityInfo(handle, SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, .., p_new_dacl, ..)
```

- 敏感子路径（`.git`、`.nova`）额外加 **DENY** ACE
- `SetEntriesInAclW` 会把新建的 deny ACE 排在 allow 之前（Windows 行为，注释里有强调）

### 4. 进程创建（`process.rs:create_process_as_user`）

```rust
// 1) Job Object：JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE，保证进程树随父死
// 2) STARTUPINFOEXW，cb = size_of::<STARTUPINFOEXW>()
si.StartupInfo.lpDesktop = "winsta0\\default";   // ← 必须有
// 3) 属性列表：
//    PROC_THREAD_ATTRIBUTE_JOB          → job handle
//    PROC_THREAD_ATTRIBUTE_HANDLE_LIST  → stdin/stdout/stderr（bInheritHandles=1）
// 4) 环境块：双 null 结尾 + CREATE_UNICODE_ENVIRONMENT
let creation_flags = CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT | CREATE_NO_WINDOW;
CreateProcessAsUserW(token, null, cmdline, null, null, 1, creation_flags, env_block, cwd, &si.StartupInfo, &mut pi);
```

**`lpDesktop` 是必须的**。Codex 源码注释原话：

> "Some processes (e.g., PowerShell) can fail with STATUS_DLL_INIT_FAILED if `lpDesktop` is not set when launching with a restricted token."

`rappct` 缺的就是这一句。

### 5. 网络

- **unelevated 模式只有弱管控**：环境变量 `CODEX_SANDBOX_NETWORK_DISABLED=1`（进程可以不理）
- 内核级要 elevated 模式：独立沙箱账户 + Windows Firewall 规则
- Nova 的对位：`NOVA_SANDBOX_NETWORK_DISABLED=1`（弱），后续再接 WFP

## 四、已验证的坑

| 现象 | 原因 | 处理 |
|---|---|---|
| `CreateProcessW` 返回 203 | 自定义环境块缺 `SystemRoot` 等必需变量 | 传入完整父环境，或至少补 SystemRoot/windir/ComSpec/PATHEXT/TEMP/TMP/PATH |
| PowerShell 启动即 0xC0000142 | 受限令牌 + `lpDesktop` 为 NULL | 显式设 `"winsta0\\default"` |
| 错误码像是随机值 | `CreateProcessW` 失败后 `GetLastError` 可能读到残留值 | 调用前 `SetLastError(0)` |
| AppContainer 报 203 | 该路线本身不适合 agent 负载 | 放弃，改用 write-restricted token |

## 五、验证方案

分两步，都能独立验证：

**第一步（token + 进程，不含 ACL）**

```rust
// restricting SIDs 只有 Logon + Everyone
// 预期：whoami 能起，但写家目录失败
```

**第二步（加 ACL）**

```rust
// 给工作区加合成 SID 的 ACE
// 预期：写工作区成功，写家目录仍失败
```

验证命令（Windows）：

```
cargo run -p nova-core --example win_sandbox_probe
```

探测三件事：写工作区、写家目录、`whoami /groups` 看 restricting SIDs。

## 六、工作量

| 部分 | 行数 | 风险 |
|---|---|---|
| SID 生成 + 持久化 | ~80 | 低 |
| 受限令牌 | ~120 | 中（API 参数多） |
| ACL 授权 | ~150 | 中（`SetEntriesInAclW` 的 RAII 清理） |
| 进程创建 | ~200 | 高（属性列表 + 生命周期） |
| 接入 `SandboxDriver` | ~60 | 低 |

合计约 500–600 行，2–3 天含调试。`CreateProcessAsUserW` 在"同用户受限令牌"下无需 `SeAssignPrimaryTokenPrivilege`——Codex 的 unelevated 模式也这么调，证明可行。
