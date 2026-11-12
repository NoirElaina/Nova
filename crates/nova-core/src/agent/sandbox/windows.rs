//! Windows 沙箱驱动：write-restricted token + 合成 SID + ACL + 私有桌面 + JobObject + ConPTY/Pipe。
//!
//! 设计（对齐 Codex `windows-sandbox-rs`）：
//! - `WRITE_RESTRICTED` 受限令牌只在写操作时校验 Restricting SID 的 ACL，读取全局系统 DLL/工具不受影响；
//! - 工作区（及沙箱专用 `TEMP`/`TMP`）在 `WorkspaceWrite` 下授予合成 SID 读写权限，并对 `.git/hooks`、
//!   `.git/config`、`.nova` 叠加写拒绝 ACE；`ReadOnly` 下使用未授权的只读 SID，禁止一切写入；
//! - 显式创建/复用授权当前 Logon SID 的私有桌面（避免受限令牌在 `user32`/DLL 初始化期因桌面 ACL 触发 `0xC0000142`）；
//! - 每个沙箱子进程绑定带 `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 的 JobObject，确保退出/取消时整棵进程树被回收；
//! - 不限制出站网络（只做文件写入与进程树隔离）。

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, SetHandleInformation, ERROR_SUCCESS, GENERIC_ALL,
    HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LUID,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, SetSecurityInfo, DENY_ACCESS,
    EXPLICIT_ACCESS_W, GRANT_ACCESS, SE_FILE_OBJECT, SE_WINDOW_OBJECT, TRUSTEE_IS_SID,
    TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, CreateRestrictedToken, CreateWellKnownSid, GetTokenInformation,
    LookupPrivilegeValueW, SetTokenInformation, TokenDefaultDacl, TokenLogonSid, TokenUser, ACL,
    DACL_SECURITY_INFORMATION, DISABLE_MAX_PRIVILEGE, LUA_TOKEN, LUID_AND_ATTRIBUTES,
    PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, SE_PRIVILEGE_ENABLED, SID_AND_ATTRIBUTES,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_ADJUST_SESSIONID, TOKEN_ALL_ACCESS, TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_GROUPS,
    TOKEN_PRIVILEGES, TOKEN_QUERY, TOKEN_USER, WinWorldSid,
};
use windows_sys::Win32::System::Console::{
    ClosePseudoConsole, CreatePseudoConsole, ResizePseudoConsole, COORD, HPCON,
};
use windows_sys::Win32::System::JobObjects::{
    CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::StationsAndDesktops::{CloseDesktop, CreateDesktopW, HDESK};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, DeleteProcThreadAttributeList, GetCurrentProcess, GetExitCodeProcess,
    InitializeProcThreadAttributeList, OpenProcessToken, UpdateProcThreadAttribute,
    WaitForSingleObject, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
    EXTENDED_STARTUPINFO_PRESENT, INFINITE, LPPROC_THREAD_ATTRIBUTE_LIST, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW,
};

use super::SandboxPolicy;

/// `WRITE_RESTRICTED` 未在 windows-sys 中导出，取 Win32 头文件定义。
const WRITE_RESTRICTED: u32 = 0x8;
/// 仅拒绝写/改/删，不拒绝读与 `SYNCHRONIZE`（对齐 Codex `STANDARD_USER_MUTATION_MASK`）。
const DENY_WRITE_MASK: u32 = 0x0002 // FILE_WRITE_DATA
    | 0x0004 // FILE_APPEND_DATA
    | 0x0010 // FILE_WRITE_EA
    | 0x0040 // FILE_DELETE_CHILD
    | 0x0100 // FILE_WRITE_ATTRIBUTES
    | 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000; // WRITE_OWNER
/// 仅阻止改名/删除 `.git` 目录自身或修改其 ACL（不继承到子项，避免与已有只读或目录语义冲突）。
const DENY_DELETE_AND_ACL_MASK: u32 = 0x0001_0000 // DELETE
    | 0x0004_0000 // WRITE_DAC
    | 0x0008_0000; // WRITE_OWNER
/// ReadOnly 模式使用的固定未授权合成 SID：仅作约束占位，绝不对任何目录授予写 ACL。
const READ_ONLY_CAP_SID: &str = "S-1-5-21-0-0-0-1";
const PROC_THREAD_ATTRIBUTE_HANDLE_LIST: usize = 0x0002_0002;
const PROC_THREAD_ATTRIBUTE_JOB_LIST: usize = 0x0002_000D;
const PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE: usize = 0x0002_0016;

/// 合成 SID 的持久化文件名（放在应用数据目录下）。
const CAP_SID_FILE: &str = "sandbox_caps.json";

fn to_wide(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// 为可写根目录生成/复用一个合成 SID。
///
/// 形如 `S-1-5-21-a-b-c-d`：格式合法、可作 ACL trustee，但**不对应任何真实账户**。
pub fn workspace_write_sid(data_dir: &Path, workspace: &Path) -> Result<String, String> {
    let path = data_dir.join(CAP_SID_FILE);
    let key = workspace
        .to_string_lossy()
        .replace('\\', "/")
        .to_ascii_lowercase();

    let mut map: std::collections::BTreeMap<String, String> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|txt| serde_json::from_str(&txt).ok())
        .unwrap_or_default();

    if let Some(existing) = map.get(&key) {
        return Ok(existing.clone());
    }

    let sid = random_cap_sid();
    map.insert(key, sid.clone());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(txt) = serde_json::to_string_pretty(&map) {
        let _ = std::fs::write(&path, txt);
    }
    Ok(sid)
}

fn random_cap_sid() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut seed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    let mut next = || {
        // xorshift64*，够用即可；SID 只需在本机唯一且稳定。
        seed ^= seed >> 12;
        seed ^= seed << 25;
        seed ^= seed >> 27;
        (seed.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 32) as u32
    };
    format!("S-1-5-21-{}-{}-{}-{}", next(), next(), next(), next())
}

/// 当前进程令牌 → 写限制令牌。
///
/// `restricting_sids` 里**不含当前用户 SID**：写操作只认这些 SID 的 ACE，
/// 所以除显式授权的目录外一律写不进去（包括自己的家目录）。读不受影响。
pub fn create_write_restricted_token(restricting_sids: &[String]) -> Result<HANDLE, String> {
    unsafe {
        let mut base: HANDLE = std::ptr::null_mut();
        let desired = TOKEN_DUPLICATE
            | TOKEN_QUERY
            | TOKEN_ASSIGN_PRIMARY
            | TOKEN_ADJUST_DEFAULT
            | TOKEN_ADJUST_SESSIONID
            | TOKEN_ADJUST_PRIVILEGES;
        if OpenProcessToken(GetCurrentProcess(), desired, &mut base) == 0 {
            return Err(format!("OpenProcessToken failed: {}", GetLastError()));
        }
        let base_guard = OwnedHandle::from_raw_handle(base as _);
        let base = base_guard.as_raw_handle() as HANDLE;

        // Everyone + 传入的合成 SID 一起作为限制 SID。
        let mut everyone = everyone_sid_bytes()?;
        let mut logon = logon_sid_bytes(base)?;
        let mut sid_buffers: Vec<Vec<u8>> = Vec::new();
        for sid in restricting_sids {
            sid_buffers.push(string_sid_bytes(sid)?);
        }

        let mut entries: Vec<SID_AND_ATTRIBUTES> = Vec::new();
        for buf in sid_buffers.iter_mut() {
            entries.push(SID_AND_ATTRIBUTES {
                Sid: buf.as_mut_ptr() as *mut _,
                Attributes: 0,
            });
        }
        // Logon SID 必须在内：会话对象（窗口站、桌面）的 ACL 只认它，
        // 缺了就进不去桌面，进程以 DLL_INIT_FAILED 结束。
        entries.push(SID_AND_ATTRIBUTES {
            Sid: logon.as_mut_ptr() as *mut _,
            Attributes: 0,
        });
        entries.push(SID_AND_ATTRIBUTES {
            Sid: everyone.as_mut_ptr() as *mut _,
            Attributes: 0,
        });

        let flags = DISABLE_MAX_PRIVILEGE | LUA_TOKEN | WRITE_RESTRICTED;
        let mut token: HANDLE = std::ptr::null_mut();
        let ok = CreateRestrictedToken(
            base,
            flags,
            0,
            std::ptr::null(),
            0,
            std::ptr::null(),
            entries.len() as u32,
            entries.as_mut_ptr(),
            &mut token,
        );
        drop(base_guard);

        if ok == 0 {
            return Err(format!("CreateRestrictedToken failed: {}", GetLastError()));
        }

        // 设置 TokenDefaultDacl 为当前 Logon SID + OWNER RIGHTS 读控制（对齐 Codex `token.rs`），
        // 并将 `SeChangeNotifyPrivilege` 重新启用，防止受限进程无法初始化 `conhost`/`user32` (`0xC0000142`)。
        if let Err(error) = set_default_dacl(token, logon.as_mut_ptr() as *mut std::ffi::c_void)
            .and_then(|()| enable_privilege(token, "SeChangeNotifyPrivilege"))
        {
            CloseHandle(token);
            return Err(error);
        }

        Ok(token)
    }
}

#[repr(C)]
struct TokenDefaultDaclInfo {
    default_dacl: *mut ACL,
}

unsafe fn set_default_dacl(
    token: HANDLE,
    logon_sid_ptr: *mut std::ffi::c_void,
) -> Result<(), String> {
    const READ_CONTROL_ACCESS: u32 = 0x0002_0000;
    let mut owner_rights = string_sid_bytes("S-1-3-4")?;
    let entries = [
        (logon_sid_ptr, GENERIC_ALL),
        (owner_rights.as_mut_ptr() as *mut std::ffi::c_void, READ_CONTROL_ACCESS),
    ]
    .map(|(sid, access)| EXPLICIT_ACCESS_W {
        grfAccessPermissions: access,
        grfAccessMode: GRANT_ACCESS,
        grfInheritance: 0,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: 0,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_UNKNOWN,
            ptstrName: sid as *mut u16,
        },
    });
    let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
    let code = SetEntriesInAclW(
        entries.len() as u32,
        entries.as_ptr(),
        std::ptr::null_mut(),
        &mut p_new_dacl,
    );
    if code != ERROR_SUCCESS {
        return Err(format!("SetEntriesInAclW(TokenDefaultDacl) failed: {code}"));
    }
    let mut info = TokenDefaultDaclInfo {
        default_dacl: p_new_dacl,
    };
    let ok = SetTokenInformation(
        token,
        TokenDefaultDacl,
        std::ptr::addr_of_mut!(info).cast(),
        std::mem::size_of::<TokenDefaultDaclInfo>() as u32,
    );
    LocalFree(p_new_dacl as *mut _);
    if ok == 0 {
        return Err(format!(
            "SetTokenInformation(TokenDefaultDacl) failed: {}",
            GetLastError()
        ));
    }
    Ok(())
}

/// 在令牌上启用单个特权。
unsafe fn enable_privilege(token: HANDLE, name: &str) -> Result<(), String> {
    let wide = to_wide(name);
    let mut luid: LUID = std::mem::zeroed();
    if LookupPrivilegeValueW(std::ptr::null(), wide.as_ptr(), &mut luid) == 0 {
        return Err(format!(
            "LookupPrivilegeValueW('{}') failed: {}",
            name,
            GetLastError()
        ));
    }

    let state = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };

    if AdjustTokenPrivileges(
        token,
        0,
        &state,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
    ) == 0
    {
        return Err(format!(
            "AdjustTokenPrivileges('{}') failed: {}",
            name,
            GetLastError()
        ));
    }
    Ok(())
}

/// 生成 Everyone SID 并返回其字节表示。
unsafe fn everyone_sid_bytes() -> Result<Vec<u8>, String> {
    let mut size: u32 = 0;
    CreateWellKnownSid(
        WinWorldSid,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        &mut size,
    );
    let mut buf = vec![0u8; size as usize];
    if CreateWellKnownSid(
        WinWorldSid,
        std::ptr::null_mut(),
        buf.as_mut_ptr() as *mut _,
        &mut size,
    ) == 0
    {
        return Err(format!("CreateWellKnownSid(Everyone) failed: {}", GetLastError()));
    }
    Ok(buf)
}

/// 从令牌中取 Logon SID（`S-1-5-5-X-Y`）。
///
/// 会话对象（窗口站、桌面）的 ACL 只认这个 SID。限制 SID 列表缺了它，
/// 进程即使指定了 `lpDesktop` 也进不去桌面，最终以 `DLL_INIT_FAILED` 结束。
unsafe fn logon_sid_bytes(token: HANDLE) -> Result<Vec<u8>, String> {
    let mut size: u32 = 0;
    GetTokenInformation(token, TokenLogonSid, std::ptr::null_mut(), 0, &mut size);
    if size == 0 {
        return Err(format!(
            "GetTokenInformation(TokenLogonSid) size failed: {}",
            GetLastError()
        ));
    }
    let mut buf = vec![0u8; size as usize];
    if GetTokenInformation(
        token,
        TokenLogonSid,
        buf.as_mut_ptr() as *mut _,
        size,
        &mut size,
    ) == 0
    {
        return Err(format!(
            "GetTokenInformation(TokenLogonSid) failed: {}",
            GetLastError()
        ));
    }

    let groups = &*(buf.as_ptr() as *const TOKEN_GROUPS);
    if groups.GroupCount == 0 {
        return Err("当前令牌不含 Logon SID".to_string());
    }
    let sid = groups.Groups[0].Sid as *const u8;
    let count = *sid.add(1) as usize;
    let len = 8 + count * 4;
    let mut out = vec![0u8; len];
    std::ptr::copy_nonoverlapping(sid, out.as_mut_ptr(), len);
    Ok(out)
}

/// 字符串 SID（`S-1-5-21-...`）→ 字节表示。
unsafe fn string_sid_bytes(sid: &str) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    let wide = to_wide(sid);
    let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    if ConvertStringSidToSidW(wide.as_ptr(), &mut ptr) == 0 {
        return Err(format!(
            "ConvertStringSidToSidW('{}') failed: {}",
            sid,
            GetLastError()
        ));
    }
    // SID 长度：8 字节头 + SubAuthorityCount * 4
    let bytes = ptr as *const u8;
    let count = *bytes.add(1) as usize;
    let len = 8 + count * 4;
    let mut out = vec![0u8; len];
    std::ptr::copy_nonoverlapping(bytes, out.as_mut_ptr(), len);
    windows_sys::Win32::Foundation::LocalFree(ptr as *mut _);
    Ok(out)
}

fn apply_path_dacl(
    path: &Path,
    cap_sid: &str,
    access_mode: i32,
    permissions: u32,
) -> Result<(), String> {
    apply_path_dacl_with_inheritance(
        path,
        cap_sid,
        access_mode,
        permissions,
        SUB_CONTAINERS_AND_OBJECTS_INHERIT,
    )
}

fn apply_path_dacl_with_inheritance(
    path: &Path,
    cap_sid: &str,
    access_mode: i32,
    permissions: u32,
    grf_inheritance: u32,
) -> Result<(), String> {
    unsafe {
        let mut sid_bytes = string_sid_bytes(cap_sid)?;
        let sid_ptr = sid_bytes.as_mut_ptr() as *mut std::ffi::c_void;
        let path_wide = to_wide(&path.to_string_lossy());

        let mut p_dacl: *mut ACL = std::ptr::null_mut();
        let mut p_sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let code = GetNamedSecurityInfoW(
            path_wide.as_ptr(),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut p_dacl,
            std::ptr::null_mut(),
            &mut p_sd,
        );
        if code != ERROR_SUCCESS {
            return Err(format!(
                "GetNamedSecurityInfoW('{}') failed: {}",
                path.display(),
                code
            ));
        }

        let explicit = EXPLICIT_ACCESS_W {
            grfAccessPermissions: permissions,
            grfAccessMode: access_mode,
            grfInheritance: grf_inheritance,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: sid_ptr as *mut u16,
            },
        };

        let mut p_new_dacl: *mut ACL = std::ptr::null_mut();
        let code = SetEntriesInAclW(1, &explicit, p_dacl, &mut p_new_dacl);
        if code != ERROR_SUCCESS {
            LocalFree(p_sd as *mut _);
            return Err(format!("SetEntriesInAclW failed: {}", code));
        }

        let code = SetNamedSecurityInfoW(
            path_wide.as_ptr() as *mut u16,
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            p_new_dacl,
            std::ptr::null_mut(),
        );

        LocalFree(p_new_dacl as *mut _);
        LocalFree(p_sd as *mut _);

        if code != ERROR_SUCCESS {
            return Err(format!(
                "SetNamedSecurityInfoW('{}') failed: {}",
                path.display(),
                code
            ));
        }
        Ok(())
    }
}

/// 给工作区目录授予合成 SID 的完全控制 ACL（可继承到子目录与新文件）。
pub fn grant_workspace_write(workspace: &Path, cap_sid: &str) -> Result<(), String> {
    apply_path_dacl(workspace, cap_sid, GRANT_ACCESS, GENERIC_ALL)
}

/// 在指定路径上给合成 SID 加 DENY 写 ACE（用于 `.git/hooks`、`.git/config`、`.nova` 等敏感路径）。
pub fn deny_write(path: &Path, cap_sid: &str) -> Result<(), String> {
    apply_path_dacl(path, cap_sid, DENY_ACCESS, DENY_WRITE_MASK)
}

/// 在目录本身加非继承 DENY `DELETE | WRITE_DAC | WRITE_OWNER` ACE（防止改名 `.git` 绕过子目录 DENY）。
pub fn deny_delete_and_acl(path: &Path, cap_sid: &str) -> Result<(), String> {
    apply_path_dacl_with_inheritance(path, cap_sid, DENY_ACCESS, DENY_DELETE_AND_ACL_MASK, 0)
}

/// Windows Job Object RAII 句柄：`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` 确保关闭或终止时清空整棵子进程树。
pub struct WinJobObject {
    handle: OwnedHandle,
}

impl WinJobObject {
    pub fn create_kill_on_close() -> Result<Self, String> {
        unsafe {
            let raw = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if raw.is_null() {
                return Err(format!("CreateJobObjectW failed: {}", GetLastError()));
            }
            let handle = OwnedHandle::from_raw_handle(raw as _);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle.as_raw_handle() as HANDLE,
                JobObjectExtendedLimitInformation,
                std::ptr::addr_of_mut!(limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if ok == 0 {
                return Err(format!(
                    "SetInformationJobObject failed: {}",
                    GetLastError()
                ));
            }
            Ok(Self { handle })
        }
    }

    pub fn terminate(&self) {
        unsafe {
            let _ = TerminateJobObject(self.handle.as_raw_handle() as HANDLE, 1);
        }
    }

    fn as_raw_handle(&self) -> HANDLE {
        self.handle.as_raw_handle() as HANDLE
    }
}

/// 等待 Win32 进程句柄结束并读取退出码（供 tokio `spawn_blocking` 调用）。
pub fn wait_process_exit(process: &OwnedHandle) -> i32 {
    unsafe {
        WaitForSingleObject(process.as_raw_handle() as HANDLE, INFINITE);
        let mut code: u32 = 1;
        GetExitCodeProcess(process.as_raw_handle() as HANDLE, &mut code);
        code as i32
    }
}

/// Windows ConPTY 句柄封装（Drop 自动关闭）。
pub struct WinConPty {
    hpc: HPCON,
}

unsafe impl Send for WinConPty {}
unsafe impl Sync for WinConPty {}

impl WinConPty {
    pub fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let size = COORD {
            X: cols.clamp(1, i16::MAX as u16) as i16,
            Y: rows.clamp(1, i16::MAX as u16) as i16,
        };
        let hr = unsafe { ResizePseudoConsole(self.hpc, size) };
        if hr < 0 {
            return Err(format!("ResizePseudoConsole failed: HRESULT 0x{hr:08x}"));
        }
        Ok(())
    }
}

impl Drop for WinConPty {
    fn drop(&mut self) {
        unsafe {
            ClosePseudoConsole(self.hpc);
        }
    }
}

pub struct SandboxedPipeSpawn {
    pub child_process: OwnedHandle,
    pub pid: u32,
    pub job: WinJobObject,
    pub stdin: std::fs::File,
    pub stdout: std::fs::File,
    pub stderr: std::fs::File,
}

pub struct SandboxedPtySpawn {
    pub child_process: OwnedHandle,
    pub pid: u32,
    pub job: WinJobObject,
    pub conpty: WinConPty,
    pub reader: std::fs::File,
    pub writer: std::fs::File,
}

struct ProcThreadAttrList {
    buf: Vec<u8>,
}

impl ProcThreadAttrList {
    fn new(
        handle_list: &[HANDLE],
        job_handles: &mut [HANDLE],
        pseudoconsole: Option<HPCON>,
    ) -> Result<Self, String> {
        let attr_count: u32 = u32::from(!job_handles.is_empty())
            + u32::from(!handle_list.is_empty())
            + u32::from(pseudoconsole.is_some());
        let mut size: usize = 0;
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), attr_count, 0, &mut size);
        }
        if size == 0 {
            return Err(format!(
                "InitializeProcThreadAttributeList(size) failed: {}",
                unsafe { GetLastError() }
            ));
        }

        let mut buf = vec![0u8; size];
        let list = buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        let ok = unsafe { InitializeProcThreadAttributeList(list, attr_count, 0, &mut size) };
        if ok == 0 {
            return Err(format!(
                "InitializeProcThreadAttributeList failed: {}",
                unsafe { GetLastError() }
            ));
        }

        let attr_list = Self { buf };
        if !handle_list.is_empty() {
            attr_list.update(
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
                handle_list.as_ptr().cast(),
                std::mem::size_of_val(handle_list),
            )?;
        }
        if !job_handles.is_empty() {
            attr_list.update(
                PROC_THREAD_ATTRIBUTE_JOB_LIST,
                job_handles.as_mut_ptr().cast(),
                std::mem::size_of_val(job_handles),
            )?;
        }
        if let Some(hpc) = pseudoconsole {
            attr_list.update(
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
                hpc as *mut std::ffi::c_void,
                std::mem::size_of::<HPCON>(),
            )?;
        }

        Ok(attr_list)
    }

    fn update(
        &self,
        attribute: usize,
        value: *const std::ffi::c_void,
        size: usize,
    ) -> Result<(), String> {
        let list = self.buf.as_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST;
        let ok = unsafe {
            UpdateProcThreadAttribute(
                list,
                0,
                attribute,
                value,
                size,
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if ok == 0 {
            return Err(format!(
                "UpdateProcThreadAttribute(0x{attribute:x}) failed: {}",
                unsafe { GetLastError() }
            ));
        }
        Ok(())
    }

    fn as_ptr(&mut self) -> LPPROC_THREAD_ATTRIBUTE_LIST {
        self.buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST
    }
}

impl Drop for ProcThreadAttrList {
    fn drop(&mut self) {
        unsafe {
            DeleteProcThreadAttributeList(self.buf.as_mut_ptr() as LPPROC_THREAD_ATTRIBUTE_LIST);
        }
    }
}

fn make_pipe(inherit_read: bool, inherit_write: bool) -> Result<(OwnedHandle, OwnedHandle), String> {
    let mut read_raw: HANDLE = std::ptr::null_mut();
    let mut write_raw: HANDLE = std::ptr::null_mut();
    let sa = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    unsafe {
        if CreatePipe(&mut read_raw, &mut write_raw, &sa, 0) == 0 {
            return Err(format!("CreatePipe failed: {}", GetLastError()));
        }
        let read = OwnedHandle::from_raw_handle(read_raw as _);
        let write = OwnedHandle::from_raw_handle(write_raw as _);
        if !inherit_read
            && SetHandleInformation(read.as_raw_handle() as HANDLE, HANDLE_FLAG_INHERIT, 0) == 0
        {
            return Err(format!(
                "SetHandleInformation(read) failed: {}",
                GetLastError()
            ));
        }
        if !inherit_write
            && SetHandleInformation(write.as_raw_handle() as HANDLE, HANDLE_FLAG_INHERIT, 0) == 0
        {
            return Err(format!(
                "SetHandleInformation(write) failed: {}",
                GetLastError()
            ));
        }
        Ok((read, write))
    }
}

fn prepare_sandbox(
    policy: SandboxPolicy,
    data_dir: &Path,
    workspace: &Path,
) -> Result<(OwnedHandle, Option<PathBuf>), String> {
    match policy {
        SandboxPolicy::WorkspaceWrite => {
            let cap_sid = workspace_write_sid(data_dir, workspace)?;
            grant_workspace_write(workspace, &cap_sid)?;
            let git_dir = workspace.join(".git");
            if git_dir.exists() {
                deny_delete_and_acl(&git_dir, &cap_sid)?;
            }
            for protected in [
                workspace.join(".git").join("hooks"),
                workspace.join(".git").join("config"),
                workspace.join(".nova"),
            ] {
                if protected.exists() {
                    deny_write(&protected, &cap_sid)?;
                }
            }
            let sandbox_tmp = data_dir.join("sandbox_tmp");
            std::fs::create_dir_all(&sandbox_tmp).map_err(|e| {
                format!("创建沙箱临时目录失败（{}）: {e}", sandbox_tmp.display())
            })?;
            grant_workspace_write(&sandbox_tmp, &cap_sid)?;
            let raw_token = create_write_restricted_token(std::slice::from_ref(&cap_sid))?;
            let token = unsafe { OwnedHandle::from_raw_handle(raw_token as _) };
            Ok((token, Some(sandbox_tmp)))
        }
        SandboxPolicy::ReadOnly => {
            let raw_token = create_write_restricted_token(&[READ_ONLY_CAP_SID.to_string()])?;
            let token = unsafe { OwnedHandle::from_raw_handle(raw_token as _) };
            Ok((token, None))
        }
        SandboxPolicy::DangerFullAccess => {
            Err("DangerFullAccess 不应进入受限令牌启动路径".to_string())
        }
    }
}

fn make_env_block(sandbox_tmp: Option<&Path>, overrides: &[(&str, &str)]) -> Vec<u16> {
    // 键按大写归一化去重，避免 Windows 上 `Path`/`PATH` 重复导致环境变量乱序。
    let mut env_map: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (k, v) in std::env::vars_os() {
        let key = k.to_string_lossy().into_owned();
        if key.is_empty() || key.contains('=') {
            continue;
        }
        let val = v.to_string_lossy().into_owned();
        env_map.insert(key.to_ascii_uppercase(), (key, val));
    }
    if let Some(tmp_dir) = sandbox_tmp {
        let tmp_str = tmp_dir.to_string_lossy().into_owned();
        env_map.insert("TEMP".to_string(), ("TEMP".to_string(), tmp_str.clone()));
        env_map.insert("TMP".to_string(), ("TMP".to_string(), tmp_str));
    }
    for (k, v) in overrides {
        env_map.insert(
            k.to_ascii_uppercase(),
            ((*k).to_string(), (*v).to_string()),
        );
    }

    let mut block = Vec::new();
    for (_, (k, v)) in env_map {
        block.extend(OsStr::new(&format!("{k}={v}")).encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

fn quote_windows_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '\x0b', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for ch in arg.chars() {
        if ch == '\\' {
            backslashes += 1;
            continue;
        }
        if ch == '"' {
            out.extend(std::iter::repeat_n('\\', backslashes * 2 + 1));
            out.push('"');
            backslashes = 0;
            continue;
        }
        if backslashes > 0 {
            out.extend(std::iter::repeat_n('\\', backslashes));
            backslashes = 0;
        }
        out.push(ch);
    }
    if backslashes > 0 {
        out.extend(std::iter::repeat_n('\\', backslashes * 2));
    }
    out.push('"');
    out
}

fn is_cmd_exe(program: &str) -> bool {
    Path::new(program)
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|stem| stem.eq_ignore_ascii_case("cmd"))
}

fn build_cmdline(program: &str, args: &[&str]) -> String {
    let mut out = quote_windows_arg(program);
    if is_cmd_exe(program) {
        if let [flag, rest] = args {
            if flag.eq_ignore_ascii_case("/c") || flag.eq_ignore_ascii_case("/k") {
                // cmd.exe does not use MSVCRT \" unescaping; /S strips strictly the
                // outermost quote pair while preserving inner quotes verbatim.
                out.push_str(" /S ");
                out.push_str(flag);
                out.push_str(" \"");
                out.push_str(rest);
                out.push('"');
                return out;
            }
        }
    }
    for arg in args {
        out.push(' ');
        out.push_str(&quote_windows_arg(arg));
    }
    out
}

struct RestrictedLaunchSpec<'a> {
    token: &'a OwnedHandle,
    job: &'a WinJobObject,
    cwd: &'a Path,
    program: &'a str,
    args: &'a [&'a str],
    env_block: &'a [u16],
    inherit_handles: bool,
    creation_flags: u32,
    stdio_handles: [HANDLE; 3],
    inherited_handle_list: &'a [HANDLE],
    pseudoconsole: Option<HPCON>,
}

struct SharedPrivateDesktop {
    _handle: usize,
    startup_name: String,
}

static PRIVATE_DESKTOP: OnceLock<Result<SharedPrivateDesktop, String>> = OnceLock::new();

fn ensure_private_desktop_name() -> Result<&'static str, String> {
    let res = PRIVATE_DESKTOP.get_or_init(|| unsafe {
        const DESKTOP_ALL_ACCESS_FLAGS: u32 = 0x000F_01FF;
        let mut base_token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut base_token) == 0 {
            return Err(format!(
                "OpenProcessToken(TOKEN_QUERY) failed: {}",
                GetLastError()
            ));
        }
        let logon_res = logon_sid_bytes(base_token);
        CloseHandle(base_token);
        let mut logon_sid = logon_res?;

        let name = format!("NovaSandboxDesktop-{:x}", std::process::id());
        let name_wide = to_wide(&name);
        let hdesk: HDESK = CreateDesktopW(
            name_wide.as_ptr(),
            std::ptr::null(),
            std::ptr::null_mut(),
            0,
            DESKTOP_ALL_ACCESS_FLAGS,
            std::ptr::null_mut(),
        );
        if hdesk.is_null() {
            return Err(format!("CreateDesktopW failed: {}", GetLastError()));
        }

        let explicit = EXPLICIT_ACCESS_W {
            grfAccessPermissions: DESKTOP_ALL_ACCESS_FLAGS,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: 0,
            Trustee: TRUSTEE_W {
                pMultipleTrustee: std::ptr::null_mut(),
                MultipleTrusteeOperation: 0,
                TrusteeForm: TRUSTEE_IS_SID,
                TrusteeType: TRUSTEE_IS_UNKNOWN,
                ptstrName: logon_sid.as_mut_ptr() as *mut u16,
            },
        };
        let mut updated_dacl: *mut ACL = std::ptr::null_mut();
        let code = SetEntriesInAclW(1, &explicit, std::ptr::null_mut(), &mut updated_dacl);
        if code != ERROR_SUCCESS {
            let _ = CloseDesktop(hdesk);
            return Err(format!("SetEntriesInAclW(desktop) failed: {code}"));
        }
        let set_code = SetSecurityInfo(
            hdesk as HANDLE,
            SE_WINDOW_OBJECT,
            DACL_SECURITY_INFORMATION,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            updated_dacl,
            std::ptr::null_mut(),
        );
        LocalFree(updated_dacl as *mut _);
        if set_code != ERROR_SUCCESS {
            let _ = CloseDesktop(hdesk);
            return Err(format!("SetSecurityInfo(desktop) failed: {set_code}"));
        }

        Ok(SharedPrivateDesktop {
            _handle: hdesk as usize,
            startup_name: format!("Winsta0\\{name}"),
        })
    });
    res.as_ref()
        .map(|d| d.startup_name.as_str())
        .map_err(Clone::clone)
}

fn launch_restricted_process(spec: RestrictedLaunchSpec<'_>) -> Result<(OwnedHandle, u32), String> {
    let mut job_handles = [spec.job.as_raw_handle()];
    let mut attr_list = ProcThreadAttrList::new(
        spec.inherited_handle_list,
        &mut job_handles,
        spec.pseudoconsole,
    )?;

    let mut cmdline = to_wide(&build_cmdline(spec.program, spec.args));
    // 对齐 Codex `desktop.rs`：使用向 Logon SID 显式赋权的私有桌面，避免受限令牌下 `0xC0000142`。
    let mut desktop = to_wide(ensure_private_desktop_name()?);
    let cwd_wide = to_wide(&spec.cwd.to_string_lossy());

    unsafe {
        let mut si: STARTUPINFOEXW = std::mem::zeroed();
        si.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        si.StartupInfo.lpDesktop = desktop.as_mut_ptr();
        si.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        si.StartupInfo.hStdInput = spec.stdio_handles[0];
        si.StartupInfo.hStdOutput = spec.stdio_handles[1];
        si.StartupInfo.hStdError = spec.stdio_handles[2];
        si.lpAttributeList = attr_list.as_ptr();

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessAsUserW(
            spec.token.as_raw_handle() as HANDLE,
            std::ptr::null(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            i32::from(spec.inherit_handles),
            spec.creation_flags | CREATE_UNICODE_ENVIRONMENT | EXTENDED_STARTUPINFO_PRESENT,
            spec.env_block.as_ptr().cast(),
            cwd_wide.as_ptr(),
            &si.StartupInfo,
            &mut pi,
        );
        if ok == 0 {
            return Err(format!(
                "CreateProcessAsUserW failed: {} | program={}",
                GetLastError(),
                spec.program
            ));
        }

        let _thread_handle = OwnedHandle::from_raw_handle(pi.hThread as _);
        let child_process = OwnedHandle::from_raw_handle(pi.hProcess as _);
        Ok((child_process, pi.dwProcessId))
    }
}

/// 在 Windows 受限令牌 + 写边界 ACL + JobObject 内以管道模式启动子进程（`tty = false`）。
pub fn spawn_sandboxed_pipe(
    policy: SandboxPolicy,
    data_dir: &Path,
    workspace: &Path,
    cwd: &Path,
    program: &str,
    args: &[&str],
    env_overrides: &[(&str, &str)],
) -> Result<SandboxedPipeSpawn, String> {
    let (token, sandbox_tmp) = prepare_sandbox(policy, data_dir, workspace)?;
    let job = WinJobObject::create_kill_on_close()?;
    let env_block = make_env_block(sandbox_tmp.as_deref(), env_overrides);

    let (stdin_read, stdin_write) = make_pipe(true, false)?;
    let (stdout_read, stdout_write) = make_pipe(false, true)?;
    let (stderr_read, stderr_write) = make_pipe(false, true)?;

    let stdio_handles = [
        stdin_read.as_raw_handle() as HANDLE,
        stdout_write.as_raw_handle() as HANDLE,
        stderr_write.as_raw_handle() as HANDLE,
    ];
    let (child_process, pid) = launch_restricted_process(RestrictedLaunchSpec {
        token: &token,
        job: &job,
        cwd,
        program,
        args,
        env_block: &env_block,
        inherit_handles: true,
        creation_flags: CREATE_NO_WINDOW,
        stdio_handles,
        inherited_handle_list: &stdio_handles,
        pseudoconsole: None,
    })?;

    unsafe {
        Ok(SandboxedPipeSpawn {
            child_process,
            pid,
            job,
            stdin: std::fs::File::from_raw_handle(stdin_write.into_raw_handle()),
            stdout: std::fs::File::from_raw_handle(stdout_read.into_raw_handle()),
            stderr: std::fs::File::from_raw_handle(stderr_read.into_raw_handle()),
        })
    }
}

/// 在 Windows 受限令牌 + 写边界 ACL + JobObject + ConPTY 内以交互 PTY 模式启动子进程（`tty = true`）。
#[allow(clippy::too_many_arguments)]
pub fn spawn_sandboxed_conpty(
    policy: SandboxPolicy,
    data_dir: &Path,
    workspace: &Path,
    cwd: &Path,
    program: &str,
    args: &[&str],
    env_overrides: &[(&str, &str)],
    cols: u16,
    rows: u16,
) -> Result<SandboxedPtySpawn, String> {
    let (token, sandbox_tmp) = prepare_sandbox(policy, data_dir, workspace)?;
    let job = WinJobObject::create_kill_on_close()?;
    let env_block = make_env_block(sandbox_tmp.as_deref(), env_overrides);

    let (pty_in_read, pty_in_write) = make_pipe(false, false)?;
    let (pty_out_read, pty_out_write) = make_pipe(false, false)?;

    let size = COORD {
        X: cols.clamp(1, i16::MAX as u16) as i16,
        Y: rows.clamp(1, i16::MAX as u16) as i16,
    };
    let mut hpc: HPCON = 0;
    let hr = unsafe {
        CreatePseudoConsole(
            size,
            pty_in_read.as_raw_handle() as HANDLE,
            pty_out_write.as_raw_handle() as HANDLE,
            0,
            &mut hpc,
        )
    };
    if hr < 0 {
        return Err(format!("CreatePseudoConsole failed: HRESULT 0x{hr:08x}"));
    }
    let conpty = WinConPty { hpc };
    drop(pty_in_read);
    drop(pty_out_write);

    let (child_process, pid) = launch_restricted_process(RestrictedLaunchSpec {
        token: &token,
        job: &job,
        cwd,
        program,
        args,
        env_block: &env_block,
        inherit_handles: false,
        creation_flags: 0,
        stdio_handles: [INVALID_HANDLE_VALUE; 3],
        inherited_handle_list: &[],
        pseudoconsole: Some(conpty.hpc),
    })?;

    unsafe {
        Ok(SandboxedPtySpawn {
            child_process,
            pid,
            job,
            conpty,
            reader: std::fs::File::from_raw_handle(pty_out_read.into_raw_handle()),
            writer: std::fs::File::from_raw_handle(pty_in_write.into_raw_handle()),
        })
    }
}

/// 诊断用：当前令牌的用户 SID（便于确认沙箱身份）。
pub fn current_user_sid() -> Result<String, String> {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut token) == 0 {
            return Err(format!("OpenProcessToken failed: {}", GetLastError()));
        }
        let mut size: u32 = 0;
        GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut size);
        let mut buf = vec![0u8; size as usize];
        if GetTokenInformation(
            token,
            TokenUser,
            buf.as_mut_ptr() as *mut _,
            size,
            &mut size,
        ) == 0
        {
            CloseHandle(token);
            return Err(format!("GetTokenInformation failed: {}", GetLastError()));
        }
        CloseHandle(token);
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        Ok(format_sid(user.User.Sid as *const u8))
    }
}

unsafe fn format_sid(sid: *const u8) -> String {
    let revision = *sid;
    let count = *sid.add(1) as usize;
    let authority = u64::from_be_bytes([
        0, 0,
        *sid.add(2), *sid.add(3), *sid.add(4), *sid.add(5), *sid.add(6), *sid.add(7),
    ]);
    let mut out = format!("S-{}-{}", revision, authority);
    for i in 0..count {
        let base = sid.add(8 + i * 4) as *const u32;
        out.push_str(&format!("-{}", *base));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn current_user_sid_is_valid_windows_sid() {
        let sid = current_user_sid().expect("current_user_sid");
        assert!(sid.starts_with("S-1-"), "unexpected sid: {sid}");
    }

    #[test]
    fn pipe_workspace_write_allows_workspace_and_blocks_outside_and_protected_subpaths() {
        let root = tempfile::tempdir().expect("tempdir");
        let data_dir = root.path().join("data");
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(workspace.join(".git").join("hooks")).expect("hooks");
        std::fs::create_dir_all(workspace.join(".nova")).expect("nova");
        std::fs::create_dir_all(&outside).expect("outside");

        let ok_file = workspace.join("allowed.txt");
        let outside_file = outside.join("blocked.txt");
        let hooks_file = workspace.join(".git").join("hooks").join("pre-commit");
        let nova_file = workspace.join(".nova").join("blocked.txt");

        let script = format!(
            "echo ok>{} && (echo no>{} 2>nul || ver>nul) && (echo no>{} 2>nul || ver>nul) && (echo no>{} 2>nul || ver>nul)",
            ok_file.display(),
            outside_file.display(),
            hooks_file.display(),
            nova_file.display(),
        );
        let spawned = spawn_sandboxed_pipe(
            SandboxPolicy::WorkspaceWrite,
            &data_dir,
            &workspace,
            &workspace,
            "C:\\Windows\\System32\\cmd.exe",
            &["/C", &script],
            &[],
        )
        .expect("spawn_sandboxed_pipe");

        drop(spawned.stdin);
        let mut out = Vec::new();
        let mut stdout = spawned.stdout;
        let _ = stdout.read_to_end(&mut out);
        let mut err_buf = Vec::new();
        let mut stderr = spawned.stderr;
        let _ = stderr.read_to_end(&mut err_buf);
        let code = wait_process_exit(&spawned.child_process);
        assert_eq!(
            code,
            0,
            "stdout={}, stderr={}",
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err_buf)
        );
        assert!(ok_file.exists(), "workspace file should be writable");
        assert!(!outside_file.exists(), "outside file must be blocked");
        assert!(!hooks_file.exists(), ".git/hooks write must be denied");
        assert!(!nova_file.exists(), ".nova write must be denied");
    }

    #[test]
    fn pipe_read_only_blocks_workspace_writes() {
        let root = tempfile::tempdir().expect("tempdir");
        let data_dir = root.path().join("data");
        let workspace = root.path().join("workspace");
        std::fs::create_dir_all(&workspace).expect("workspace");

        let target = workspace.join("ro_blocked.txt");
        let spawned = spawn_sandboxed_pipe(
            SandboxPolicy::ReadOnly,
            &data_dir,
            &workspace,
            &workspace,
            "C:\\Windows\\System32\\cmd.exe",
            &["/C", &format!("echo blocked>\"{}\" 2>nul", target.display())],
            &[],
        )
        .expect("spawn readonly pipe");

        drop(spawned.stdin);
        let _ = wait_process_exit(&spawned.child_process);
        assert!(!target.exists(), "ReadOnly sandbox must block workspace writes");
    }

    #[test]
    fn conpty_workspace_write_allows_workspace_and_blocks_outside() {
        let root = tempfile::tempdir().expect("tempdir");
        let data_dir = root.path().join("data");
        let workspace = root.path().join("workspace");
        let outside = root.path().join("outside");
        std::fs::create_dir_all(&workspace).expect("workspace");
        std::fs::create_dir_all(&outside).expect("outside");

        let ok_file = workspace.join("pty_ok.txt");
        let outside_file = outside.join("pty_blocked.txt");
        let script = format!(
            "echo pty_ok>{} && (echo pty_no>{} 2>nul || ver>nul)",
            ok_file.display(),
            outside_file.display(),
        );
        let spawned = spawn_sandboxed_conpty(
            SandboxPolicy::WorkspaceWrite,
            &data_dir,
            &workspace,
            &workspace,
            "C:\\Windows\\System32\\cmd.exe",
            &["/C", &script],
            &[],
            120,
            30,
        )
        .expect("spawn_sandboxed_conpty");

        let mut reader = spawned.reader;
        let reader_thread = std::thread::spawn(move || {
            let mut out = Vec::new();
            let _ = reader.read_to_end(&mut out);
            out
        });
        let code = wait_process_exit(&spawned.child_process);
        drop(spawned.conpty);
        drop(spawned.writer);
        let _ = reader_thread.join();
        assert_eq!(code, 0);
        assert!(ok_file.exists(), "ConPTY workspace file should be writable");
        assert!(!outside_file.exists(), "ConPTY outside file must be blocked");
    }

    #[test]
    fn cmd_quoted_paths_with_spaces_and_job_terminate_work() {
        let root = tempfile::tempdir().expect("tempdir");
        let data_dir = root.path().join("data");
        let workspace = root.path().join("space dir");
        std::fs::create_dir_all(&workspace).expect("workspace");

        let ok_file = workspace.join("quoted file.txt");
        let script = format!("echo quoted_ok>\"{}\"", ok_file.display());
        let spawned = spawn_sandboxed_pipe(
            SandboxPolicy::WorkspaceWrite,
            &data_dir,
            &workspace,
            &workspace,
            "C:\\Windows\\System32\\cmd.exe",
            &["/C", &script],
            &[],
        )
        .expect("spawn_sandboxed_pipe");
        drop(spawned.stdin);
        let code = wait_process_exit(&spawned.child_process);
        assert_eq!(code, 0);
        assert!(ok_file.exists(), "quoted path inside workspace should be written");

        // Verify WinJobObject terminates a long-lived child process immediately.
        let sleeper = spawn_sandboxed_pipe(
            SandboxPolicy::WorkspaceWrite,
            &data_dir,
            &workspace,
            &workspace,
            "C:\\Windows\\System32\\cmd.exe",
            &["/C", "ping -n 30 127.0.0.1 >nul"],
            &[],
        )
        .expect("spawn sleeper");
        let start = std::time::Instant::now();
        sleeper.job.terminate();
        let sleeper_code = wait_process_exit(&sleeper.child_process);
        assert_ne!(sleeper_code, 0);
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "job terminate should kill sleeper promptly"
        );
    }
}
