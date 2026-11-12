//! 命令执行沙箱策略。
//!
//! 这里定义两个正交轴中的第一个：
//! - `SandboxPolicy`：命令能碰到什么（文件系统 / 网络）——本模块
//! - `ApprovalPolicy`：什么时候询问用户——见 `agent::permissions`
//!
//! 两者独立配置，支持灵活组合。
//!
//! 驱动现状：
//! - Linux：`bubblewrap`（若可用），完整支持三档策略。
//! - 其它平台：`NoSandbox`，拒绝执行需要隔离的策略（fail-closed），
//!   而不是静默退化成无隔离——否则用户会以为命令被限制了，实际没有。

use std::path::Path;

use serde::{Deserialize, Serialize};

#[cfg(windows)]
pub mod windows;

/// 命令可触及的资源边界。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum SandboxPolicy {
    /// 只读：可读工作区与系统，不可写任何位置（不限制联网）。
    ReadOnly,
    /// 工作区可写：可读全局，只能写工作区目录及必要临时目录（不限制联网）。
    WorkspaceWrite,
    /// 不限制：命令以当前用户完整权限执行。
    #[default]
    DangerFullAccess,
}

impl SandboxPolicy {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().replace('_', "-").as_str() {
            "read-only" | "readonly" => Some(Self::ReadOnly),
            "workspace-write" | "workspacewrite" => Some(Self::WorkspaceWrite),
            "danger-full-access" | "dangerfullaccess" | "full-access" | "unrestricted" | "none" => {
                Some(Self::DangerFullAccess)
            }
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
    ) -> Result<WrappedCommand, crate::NovaCoreError>;
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
    ) -> Result<WrappedCommand, crate::NovaCoreError> {
        if ctx.policy.is_enforcing() {
            return Err(crate::NovaCoreError::sandbox(
                self.name(),
                format!(
                    "沙箱策略 '{}' 需要平台驱动，当前平台只支持 'danger-full-access'。\
                     已拒绝在无隔离的情况下执行命令。",
                    ctx.policy.as_str()
                ),
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
    ) -> Result<WrappedCommand, crate::NovaCoreError> {
        if !ctx.policy.is_enforcing() {
            return Ok(WrappedCommand {
                program: program.to_string(),
                args: args.to_vec(),
                describe: "未启用沙箱：命令以当前用户完整权限执行".to_string(),
            });
        }

        let bwrap = find_in_path("bwrap").ok_or_else(|| {
            crate::NovaCoreError::sandbox(
                self.name(),
                "找不到 bubblewrap（bwrap）。请先安装：apt install bubblewrap / dnf install bubblewrap。\
                 在未隔离的情况下拒绝执行命令。",
            )
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

        // 工作区可写：只把会话目录重新 bind 成读写，并对敏感子路径补只读覆盖。
        if matches!(ctx.policy, SandboxPolicy::WorkspaceWrite) {
            if let Some(workspace) = ctx.workspace {
                let path = workspace.to_string_lossy().to_string();
                out.extend(["--bind".into(), path.clone(), path]);
                for protected in [
                    workspace.join(".git").join("hooks"),
                    workspace.join(".git").join("config"),
                    workspace.join(".nova"),
                ] {
                    if protected.exists() {
                        let p = protected.to_string_lossy().to_string();
                        out.extend(["--ro-bind".into(), p.clone(), p]);
                    }
                }
            }
        }

        // /proc 按需挂载：pid namespace 已隔离，这里给进程一个自己的 /proc（不隔离网络）。
        out.extend(["--proc".into(), "/proc".into()]);

        out.push("--".into());
        out.push(program.to_string());
        out.extend(args.iter().cloned());

        Ok(WrappedCommand {
            program: bwrap,
            args: out,
            describe: format!(
                "linux-bubblewrap（policy={}，仅文件写入与进程命名空间隔离：{}，不限联网）",
                ctx.policy.as_str(),
                if matches!(ctx.policy, SandboxPolicy::WorkspaceWrite) {
                    ctx.workspace
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "(未指定)".to_string())
                } else {
                    "(只读)".to_string()
                }
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
/// Windows 驱动声明：通过受限令牌 + 合成 Capability SID ACL + JobObject + ConPTY/Pipe
/// 在进程创建期完成隔离（由 `sandbox::windows` 执行，不通过 CLI 包装）。
#[cfg(target_os = "windows")]
pub struct WindowsRestrictedSandbox;

#[cfg(target_os = "windows")]
impl SandboxDriver for WindowsRestrictedSandbox {
    fn name(&self) -> &'static str {
        "windows-restricted"
    }

    fn is_enforcing(&self) -> bool {
        true
    }

    fn wrap(
        &self,
        program: &str,
        args: &[String],
        ctx: &SandboxContext<'_>,
    ) -> Result<WrappedCommand, crate::NovaCoreError> {
        Ok(WrappedCommand {
            program: program.to_string(),
            args: args.to_vec(),
            describe: status_for_policy(ctx.policy).summary,
        })
    }
}

/// 解析沙箱策略：环境变量 `NOVA_SANDBOX_POLICY` 优先，其次设置项，缺省 `danger-full-access`。
pub fn resolve_sandbox_policy(configured: Option<&str>) -> SandboxPolicy {
    if let Some(from_env) = std::env::var("NOVA_SANDBOX_POLICY")
        .ok()
        .and_then(|raw| SandboxPolicy::parse(&raw))
    {
        return from_env;
    }
    configured
        .and_then(SandboxPolicy::parse)
        .unwrap_or_default()
}

pub fn active_policy() -> SandboxPolicy {
    resolve_sandbox_policy(None)
}

/// 读取当前全局设置下的生效沙箱策略（环境变量 `NOVA_SANDBOX_POLICY` 可覆盖）。
pub fn effective_sandbox_policy(core: &crate::core::Core) -> SandboxPolicy {
    let configured = crate::services::settings::load_settings(core)
        .ok()
        .map(|s| s.sandbox_policy);
    resolve_sandbox_policy(configured.as_deref())
}

pub fn active_driver() -> &'static dyn SandboxDriver {
    #[cfg(target_os = "linux")]
    {
        &BubblewrapSandbox
    }
    #[cfg(target_os = "windows")]
    {
        &WindowsRestrictedSandbox
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    {
        &NoSandbox
    }
}

/// 构造给定策略下当前平台如实的沙箱状态说明。
pub fn status_for_policy(policy: SandboxPolicy) -> nova_protocol::misc::SandboxStatus {
    if !policy.is_enforcing() {
        return nova_protocol::misc::SandboxStatus {
            driver: "none".to_string(),
            policy: policy.as_str().to_string(),
            enforcing: false,
            network_restricted: false,
            summary: "未开启沙箱：命令以当前用户完整权限执行（不限文件写入，不限联网）".to_string(),
        };
    }

    #[cfg(target_os = "windows")]
    {
        nova_protocol::misc::SandboxStatus {
            driver: "windows-restricted".to_string(),
            policy: policy.as_str().to_string(),
            enforcing: true,
            network_restricted: false,
            summary: format!(
                "windows-restricted（{}：仅文件写入与进程树隔离，不限联网）",
                policy.as_str()
            ),
        }
    }

    #[cfg(target_os = "linux")]
    {
        let has_bwrap = find_in_path("bwrap").is_some();
        nova_protocol::misc::SandboxStatus {
            driver: "linux-bubblewrap".to_string(),
            policy: policy.as_str().to_string(),
            enforcing: has_bwrap,
            network_restricted: false,
            summary: if has_bwrap {
                format!(
                    "linux-bubblewrap（{}：仅文件写入与进程命名空间隔离，不限联网）",
                    policy.as_str()
                )
            } else {
                "linux-bubblewrap（未检测到 bwrap，请先安装 bubblewrap；不限联网）".to_string()
            },
        }
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        nova_protocol::misc::SandboxStatus {
            driver: "none".to_string(),
            policy: policy.as_str().to_string(),
            enforcing: false,
            network_restricted: false,
            summary: "未开启沙箱（当前平台未实现写入隔离）".to_string(),
        }
    }
}

/// 读取当前核心配置的真实沙箱状态。
pub fn active_status(core: &crate::core::Core) -> nova_protocol::misc::SandboxStatus {
    status_for_policy(effective_sandbox_policy(core))
}

/// 把 shell 启动命令包装进沙箱（显式策略版本；Linux 使用 bubblewrap）。
pub fn wrap_shell_with_policy(
    policy: SandboxPolicy,
    program: &str,
    args: &[&str],
    workspace: Option<&str>,
) -> Result<WrappedCommand, crate::NovaCoreError> {
    let ctx = SandboxContext {
        policy,
        workspace: workspace.map(Path::new),
    };
    let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
    active_driver().wrap(program, &owned, &ctx)
}

/// 把 shell 启动命令包装进沙箱。`workspace` 为会话可写目录。
pub fn wrap_shell(
    program: &str,
    args: &[&str],
    workspace: Option<&str>,
) -> Result<WrappedCommand, crate::NovaCoreError> {
    wrap_shell_with_policy(active_policy(), program, args, workspace)
}

/// 供日志/UI 展示当前隔离状态。
pub fn describe_active() -> String {
    status_for_policy(active_policy()).summary
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
            assert!(matches!(
                error,
                crate::NovaCoreError::Sandbox { driver: "none", .. }
            ));
            let msg = error.to_string();
            assert!(msg.contains(policy.as_str()), "错误信息应包含策略名: {msg}");
        }
    }

    #[test]
    fn bubblewrap_binds_workspace_without_net_isolation_and_protects_sensitive_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(dir.path().join(".git").join("hooks")).expect("hooks dir");
        std::fs::write(dir.path().join(".git").join("config"), "[core]\n").expect("git config");
        std::fs::create_dir_all(dir.path().join(".nova")).expect("nova dir");

        let ctx = SandboxContext {
            policy: SandboxPolicy::WorkspaceWrite,
            workspace: Some(dir.path()),
        };
        match BubblewrapSandbox.wrap("sh", &["-s".to_string()], &ctx) {
            Ok(wrapped) => {
                assert!(!wrapped.args.iter().any(|a| a == "--unshare-net"));
                assert!(wrapped.args.iter().any(|a| a == "--ro-bind"));
                assert!(wrapped.args.windows(2).any(|w| w[0] == "--bind"));
                assert!(wrapped.args.windows(2).any(|w| {
                    w[0] == "--ro-bind" && w[1].ends_with("hooks")
                }));
                assert!(wrapped.args.windows(2).any(|w| {
                    w[0] == "--ro-bind" && w[1].ends_with(".nova")
                }));
                assert!(wrapped.args.iter().any(|a| a == "--die-with-parent"));
            }
            // 未安装 bwrap 时必须 fail-closed，而不是返回未包装命令。
            Err(error) => {
                assert!(matches!(
                    error,
                    crate::NovaCoreError::Sandbox {
                        driver: "bubblewrap",
                        ..
                    }
                ));
                let msg = error.to_string();
                assert!(msg.contains("bubblewrap"), "应提示安装: {msg}");
            }
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
