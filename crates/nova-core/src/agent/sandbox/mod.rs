//! 命令执行沙箱策略（语义对齐 Codex 的 `SandboxPolicy`）。
//!
//! 这里定义**两个正交轴中的第一个**：
//! - `SandboxPolicy`：命令**能碰到什么**（文件系统 / 网络）——本模块
//! - `ApprovalPolicy`：什么时候**问用户**——见 `agent::permissions`
//!
//! 两者独立配置，与 Codex 的 `--sandbox` × `--ask-for-approval` 一致。
//!
//! 驱动现状：
//! - Linux：`bubblewrap`（若可用），完整支持三档策略。
//! - 其它平台：`NoSandbox`，**拒绝**执行需要隔离的策略（fail-closed），
//!   而不是静默退化成无隔离——否则用户会以为命令被限制了，实际没有。

use std::path::Path;

use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub mod windows;

/// 命令可触及的资源边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SandboxPolicy {
    /// 只读：可读工作区与系统，不可写任何位置，禁止联网。
    ReadOnly,
    /// 工作区可写：可读全局，只能写工作区目录，默认禁止联网。
    WorkspaceWrite,
    /// 不限制：命令以当前用户完整权限执行。
    DangerFullAccess,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self::DangerFullAccess
    }
}

impl SandboxPolicy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "read-only" | "readonly" => Some(Self::ReadOnly),
            "workspace-write" | "workspacewrite" => Some(Self::WorkspaceWrite),
            "danger-full-access" | "dangerfullaccess" | "none" => Some(Self::DangerFullAccess),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::ReadOnly => "read-only",
            Self::WorkspaceWrite => "workspace-write",
            Self::DangerFullAccess => "danger-full-access",
        }
    }

    /// 是否需要真实隔离。`DangerFullAccess` 之外都必须有平台驱动。
    pub fn is_enforcing(&self) -> bool {
        !matches!(self, Self::DangerFullAccess)
    }

    /// 该策略是否允许出站网络。
    pub fn allows_network(&self) -> bool {
        matches!(self, Self::DangerFullAccess)
    }
}

/// 驱动执行约束所需的上文。
pub struct SandboxContext<'a> {
    pub policy: SandboxPolicy,
    /// 当前会话工作区：`WorkspaceWrite` 下唯一允许写入的位置。
    pub workspace: Option<&'a Path>,
}

/// 把 shell 命令包装进沙箱后的结果。
#[derive(Debug)]
pub struct WrappedCommand {
    pub program: String,
    pub args: Vec<String>,
    /// 实际生效的隔离说明，用于日志与 UI 呈现。
    pub describe: String,
}

/// 平台沙箱驱动。
pub trait SandboxDriver: Send + Sync {
    fn name(&self) -> &'static str;

    /// 是否具备真实隔离能力（`false` 时只接受 `DangerFullAccess`）。
    fn is_enforcing(&self) -> bool;

    /// 把原始命令包装成沙箱内的命令。
    ///
    /// 返回 `Err` 表示沙箱无法建立，调用方**必须**放弃执行（fail-closed），
    /// 不允许降级为无隔离继续跑。
    fn wrap(
        &self,
        program: &str,
        args: &[String],
        ctx: &SandboxContext<'_>,
    ) -> Result<WrappedCommand, String>;
}

/// 无隔离驱动：仅允许 `DangerFullAccess`。
pub struct NoSandbox;

impl SandboxDriver for NoSandbox {
    fn name(&self) -> &'static str {
        "none"
    }

    fn is_enforcing(&self) -> bool {
        false
    }

    fn wrap(
        &self,
        program: &str,
        args: &[String],
        ctx: &SandboxContext<'_>,
    ) -> Result<WrappedCommand, String> {
        if ctx.policy.is_enforcing() {
            return Err(format!(
                "沙箱策略 '{}' 需要平台驱动，当前平台只支持 'danger-full-access'。\
                 已拒绝在无隔离的情况下执行命令。",
                ctx.policy.as_str()
            ));
        }
        Ok(WrappedCommand {
            program: program.to_string(),
            args: args.to_vec(),
            describe: "未启用沙箱：命令以当前用户完整权限执行".to_string(),
        })
    }
}

/// Linux 驱动：bubblewrap。
///
/// 代码在所有平台编译（只做参数拼装，不依赖平台 API），
/// 但只在 Linux 上被选用，且运行期要求 `bwrap` 存在。
pub struct BubblewrapSandbox;

impl SandboxDriver for BubblewrapSandbox {
    fn name(&self) -> &'static str {
        "bubblewrap"
    }

    fn is_enforcing(&self) -> bool {
        true
    }

    fn wrap(
        &self,
        program: &str,
        args: &[String],
        ctx: &SandboxContext<'_>,
    ) -> Result<WrappedCommand, String> {
        if !ctx.policy.is_enforcing() {
            return Ok(WrappedCommand {
                program: program.to_string(),
                args: args.to_vec(),
                describe: "未启用沙箱：命令以当前用户完整权限执行".to_string(),
            });
        }

        let bwrap = find_in_path("bwrap").ok_or_else(|| {
            "找不到 bubblewrap（bwrap）。请先安装：apt install bubblewrap / dnf install bubblewrap。\
             在未隔离的情况下拒绝执行命令。"
                .to_string()
        })?;

        let mut out: Vec<String> = vec![
            // 整个根文件系统只读挂载。
            "--ro-bind".into(),
            "/".into(),
            "/".into(),
            // --dev 必须在可写根之前挂载，否则显式的 /dev/* 绑定会被覆盖。
            "--dev".into(),
            "/dev".into(),
            // 恢复共享内存，否则 Chrome / Postgres 一类程序初始化失败。
            "--bind-try".into(),
            "/dev/shm".into(),
            "/dev/shm".into(),
            // 独立 user / ipc / pid 命名空间：不给宿主进程与 IPC 留可见性。
            "--unshare-user".into(),
            "--unshare-ipc".into(),
            "--unshare-pid".into(),
            // 临时目录单独给一块可写的，避免污染宿主 /tmp。
            "--tmpfs".into(),
            "/tmp".into(),
            // 父进程退出时一并结束，避免孤儿。
            "--die-with-parent".into(),
            // 断开控制终端，防止 TIOCSTI 注入。
            "--new-session".into(),
            // 丢弃全部 capability，降权到底。
            "--cap-drop".into(),
            "ALL".into(),
        ];

        // 工作区可写：只把会话目录重新 bind 成读写。
        if matches!(ctx.policy, SandboxPolicy::WorkspaceWrite) {
            if let Some(workspace) = ctx.workspace {
                let path = workspace.to_string_lossy().to_string();
                out.extend(["--bind".into(), path.clone(), path]);
            }
        }

        // 网络：非 full-access 默认全部隔离。
        if !ctx.policy.allows_network() {
            out.push("--unshare-net".into());
        }

        // /proc 按需挂载：pid namespace 已隔离，这里给进程一个自己的 /proc。
        out.extend(["--proc".into(), "/proc".into()]);

        out.push("--".into());
        out.push(program.to_string());
        out.extend(args.iter().cloned());

        Ok(WrappedCommand {
            program: bwrap,
            args: out,
            describe: format!(
                "bubblewrap 沙箱：策略={}, 可写={}, 网络={}",
                ctx.policy.as_str(),
                if matches!(ctx.policy, SandboxPolicy::WorkspaceWrite) {
                    ctx.workspace
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(未指定)".to_string())
                } else {
                    "(只读)".to_string()
                },
                if ctx.policy.allows_network() { "允许" } else { "隔离" }
            ),
        })
    }
}

fn find_in_path(binary: &str) -> Option<String> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(binary))
        .find(|candidate| is_executable(candidate))
        .map(|p| p.to_string_lossy().to_string())
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        false
    }
}

/// 当前生效的策略：默认 `danger-full-access`，可用 `NOVA_SANDBOX_POLICY` 覆盖。
///
/// 默认保持不隔离，是为了不改变既有行为；要启用隔离需显式设置，例如
/// `NOVA_SANDBOX_POLICY=workspace-write`。
pub fn active_policy() -> SandboxPolicy {
    std::env::var("NOVA_SANDBOX_POLICY")
        .ok()
        .and_then(|raw| SandboxPolicy::parse(&raw))
        .unwrap_or_default()
}

pub fn active_driver() -> &'static dyn SandboxDriver {
    #[cfg(target_os = "linux")]
    {
        &BubblewrapSandbox
    }
    #[cfg(not(target_os = "linux"))]
    {
        &NoSandbox
    }
}

/// 把 shell 启动命令包装进沙箱。`workspace` 为会话可写目录。
pub fn wrap_shell(
    program: &str,
    args: &[&str],
    workspace: Option<&str>,
) -> Result<WrappedCommand, String> {
    let policy = active_policy();
    let ctx = SandboxContext {
        policy,
        workspace: workspace.map(Path::new),
    };
    let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    active_driver().wrap(program, &owned, &ctx)
}

/// 供日志/UI 展示当前隔离状态。
pub fn describe_active() -> String {
    let policy = active_policy();
    if !policy.is_enforcing() {
        return "未启用沙箱：命令以当前用户完整权限执行（含文件系统与网络）".to_string();
    }
    let ctx = SandboxContext {
        policy,
        workspace: None,
    };
    let args: Vec<String> = Vec::new();
    match active_driver().wrap("sh", &args, &ctx) {
        Ok(wrapped) => wrapped.describe,
        Err(error) => format!("沙箱不可用：{}", error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn policy_parses_all_spellings() {
        assert_eq!(SandboxPolicy::parse("read-only"), Some(SandboxPolicy::ReadOnly));
        assert_eq!(
            SandboxPolicy::parse("WORKSPACE_WRITE"),
            Some(SandboxPolicy::WorkspaceWrite)
        );
        assert_eq!(
            SandboxPolicy::parse("danger-full-access"),
            Some(SandboxPolicy::DangerFullAccess)
        );
        assert_eq!(SandboxPolicy::parse("bogus"), None);
        assert_eq!(SandboxPolicy::default(), SandboxPolicy::DangerFullAccess);
    }

    #[test]
    fn no_sandbox_passes_through_danger_full_access() {
        let ctx = SandboxContext {
            policy: SandboxPolicy::DangerFullAccess,
            workspace: None,
        };
        let wrapped = NoSandbox.wrap("sh", &["-s".to_string()], &ctx).unwrap();
        assert_eq!(wrapped.program, "sh");
        assert_eq!(wrapped.args, vec!["-s".to_string()]);
    }

    #[test]
    fn no_sandbox_refuses_enforcing_policy() {
        // 关键契约：没有隔离能力时拒绝执行，而不是静默降级。
        for policy in [SandboxPolicy::ReadOnly, SandboxPolicy::WorkspaceWrite] {
            let ctx = SandboxContext {
                policy,
                workspace: Some(Path::new("/tmp/ws")),
            };
            let error = NoSandbox.wrap("sh", &[], &ctx).unwrap_err();
            assert!(error.contains(policy.as_str()), "错误信息应包含策略名: {error}");
        }
    }

    #[test]
    fn bubblewrap_isolates_network_and_binds_workspace() {
        let ctx = SandboxContext {
            policy: SandboxPolicy::WorkspaceWrite,
            workspace: Some(Path::new("/tmp/ws")),
        };
        match BubblewrapSandbox.wrap("sh", &["-s".to_string()], &ctx) {
            Ok(wrapped) => {
                assert!(wrapped.args.iter().any(|a| a == "--unshare-net"));
                assert!(wrapped.args.iter().any(|a| a == "--ro-bind"));
                assert!(wrapped.args.windows(2).any(|w| w[0] == "--bind"));
                assert!(wrapped.args.iter().any(|a| a == "--die-with-parent"));
            }
            // 未安装 bwrap 时必须 fail-closed，而不是返回未包装命令。
            Err(error) => assert!(error.contains("bubblewrap"), "应提示安装: {error}"),
        }
    }

    #[test]
    fn read_only_never_binds_workspace_writable() {
        let ctx = SandboxContext {
            policy: SandboxPolicy::ReadOnly,
            workspace: Some(Path::new("/tmp/ws")),
        };
        if let Ok(wrapped) = BubblewrapSandbox.wrap("sh", &[], &ctx) {
            assert!(!wrapped.args.iter().any(|a| a == "--bind"));
        }
    }
}
