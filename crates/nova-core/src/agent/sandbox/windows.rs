//! Windows 沙箱：write-restricted token + 合成 SID + ACL。
//!
//! 结构对照 `codex-rs/windows-sandbox-rs`（Apache-2.0）：
//! - `token.rs`   → [`create_write_restricted_token`]
//! - `process.rs` → [`spawn_restricted`]
//! - `cap.rs`     → [`workspace_write_sid`]
//!
//! 与 AppContainer 的关键区别：**读不受限**。`WRITE_RESTRICTED` 令牌只在写操作时
//! 额外要求「限制 SID 列表中至少一个对该对象有写权限」，因此 agent 能读全局、
//! 只能写显式授权的目录——这正是 `workspace-write` 想要的形状。

#![cfg(windows)]

use std::ffi::OsStr;
use std::os::windows::ffi::OsStrExt;
use std::path::Path;

use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, LocalFree, ERROR_SUCCESS, GENERIC_ALL, HANDLE, LUID,
};
use windows_sys::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, GRANT_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_SID, TRUSTEE_IS_UNKNOWN, TRUSTEE_W,
};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, CreateRestrictedToken, CreateWellKnownSid, GetTokenInformation,
    LookupPrivilegeValueW, TokenLogonSid, TokenUser, ACL, DACL_SECURITY_INFORMATION,
    DISABLE_MAX_PRIVILEGE, LUA_TOKEN, LUID_AND_ATTRIBUTES, PSECURITY_DESCRIPTOR,
    SE_PRIVILEGE_ENABLED, SID_AND_ATTRIBUTES, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_ALL_ACCESS,
    TOKEN_GROUPS, TOKEN_PRIVILEGES, TOKEN_USER, WinWorldSid,
};
use windows_sys::Win32::System::Threading::{
    CreateProcessAsUserW, GetCurrentProcess, GetExitCodeProcess, OpenProcessToken,
    WaitForSingleObject, CREATE_NO_WINDOW, PROCESS_INFORMATION, STARTUPINFOW,
};

/// `WRITE_RESTRICTED` 未在 windows-sys 中导出，取 Win32 头文件定义。
const WRITE_RESTRICTED: u32 = 0x8;

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
/// 与 Codex 的 `make_random_cap_sid_string` 一致。
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
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut base) == 0 {
            return Err(format!("OpenProcessToken failed: {}", GetLastError()));
        }

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
        CloseHandle(base);

        if ok == 0 {
            return Err(format!("CreateRestrictedToken failed: {}", GetLastError()));
        }

        // DISABLE_MAX_PRIVILEGE 会把 SeChangeNotifyPrivilege 一并去掉。
        // 没有它，进程无法通过"遍历检查"，连 System32 下的 DLL 都加载不了，
        // 表现为 0xC0000142 (STATUS_DLL_INIT_FAILED)。Codex 同样重新启用它。
        if let Err(error) = enable_privilege(token, "SeChangeNotifyPrivilege") {
            CloseHandle(token);
            return Err(error);
        }

        Ok(token)
    }
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

    let mut state = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };

    if AdjustTokenPrivileges(
        token,
        0,
        &mut state,
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

/// 给工作区目录授予合成 SID 完全控制（继承到子目录）。
///
/// 这是 `workspace-write` 能真正写文件的前提：write-restricted 令牌在写操作时
/// 只认 restricting SID 的 ACE，没有这条授权，工作区同样不可写。
pub fn grant_workspace_write(workspace: &Path, cap_sid: &str) -> Result<(), String> {
    unsafe {
        let mut sid_bytes = string_sid_bytes(cap_sid)?;
        let sid_ptr = sid_bytes.as_mut_ptr() as *mut std::ffi::c_void;
        let path_wide = to_wide(&workspace.to_string_lossy());

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
                workspace.display(),
                code
            ));
        }

        let explicit = EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: GRANT_ACCESS,
            grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
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
                workspace.display(),
                code
            ));
        }
        Ok(())
    }
}

/// 在指定路径上给合成 SID 加 DENY 写 ACE（用于 `.git` 等敏感目录）。
pub fn deny_write(path: &Path, cap_sid: &str) -> Result<(), String> {
    use windows_sys::Win32::Security::Authorization::DENY_ACCESS;

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
            return Err(format!("GetNamedSecurityInfoW failed: {}", code));
        }

        let explicit = EXPLICIT_ACCESS_W {
            grfAccessPermissions: GENERIC_ALL,
            grfAccessMode: DENY_ACCESS,
            grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
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
            return Err(format!("SetEntriesInAclW(deny) failed: {}", code));
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
            return Err(format!("SetNamedSecurityInfoW(deny) failed: {}", code));
        }
        Ok(())
    }
}

/// 用受限令牌启动进程，返回退出码。
///
/// 关键点：`lpDesktop` 必须显式设置。Codex 源码注释指出，受限令牌下若不设
/// `lpDesktop`，PowerShell 等进程会以 `STATUS_DLL_INIT_FAILED` 失败。
pub fn spawn_restricted(
    token: HANDLE,
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
    timeout_ms: u32,
) -> Result<i32, String> {
    unsafe {
        let mut cmdline = to_wide(&build_cmdline(program, args));
        let desktop = to_wide("winsta0\\default");
        // 工作目录必须对受限令牌可读；继承父进程 cwd 时，如果 cwd 不可访问，
        // 进程会在 DLL 初始化阶段直接失败（0xC0000142）。
        let cwd_wide = to_wide(
            &cwd.map(|p| p.to_path_buf())
                .unwrap_or_else(|| std::path::PathBuf::from("C:\\Windows\\System32"))
                .to_string_lossy(),
        );

        let mut si: STARTUPINFOW = std::mem::zeroed();
        si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
        si.lpDesktop = desktop.as_ptr() as *mut u16;

        let mut pi: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessAsUserW(
            token,
            std::ptr::null(),
            cmdline.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            0,
            std::ptr::null(), // 继承父环境，避免自定义环境块引发 203
            cwd_wide.as_ptr(),
            &si,
            &mut pi,
        );

        if ok == 0 {
            return Err(format!(
                "CreateProcessAsUserW failed: {} | program={}",
                GetLastError(),
                program
            ));
        }

        WaitForSingleObject(pi.hProcess, timeout_ms);
        let mut code: u32 = 0;
        GetExitCodeProcess(pi.hProcess, &mut code);
        CloseHandle(pi.hProcess);
        CloseHandle(pi.hThread);
        Ok(code as i32)
    }
}

fn build_cmdline(program: &str, args: &[&str]) -> String {
    let mut out = String::from(program);
    for arg in args {
        out.push(' ');
        if arg.contains(' ') {
            out.push('"');
            out.push_str(arg);
            out.push('"');
        } else {
            out.push_str(arg);
        }
    }
    out
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
