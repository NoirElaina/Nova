//! Windows UDS 私有目录创建、只读安全校验及同用户非提权对端认证。

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, OwnedHandle, RawSocket};
use std::path::{Path, PathBuf};
use std::ptr;

use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, LocalFree};
use windows_sys::Win32::Networking::WinSock::{
    SIO_AF_UNIX_GETPEERPID, SOCKET_ERROR, WSAGetLastError, WSAIoctl,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo,
    SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACE_HEADER, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid,
    GetAce, GetSecurityDescriptorControl, GetTokenInformation, OBJECT_INHERIT_ACE,
    OWNER_SECURITY_INFORMATION, SE_DACL_PROTECTED, SECURITY_ATTRIBUTES, TOKEN_ELEVATION,
    TOKEN_QUERY, TOKEN_USER, TokenElevation, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, FILE_ALL_ACCESS,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS,
    FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
    FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING, READ_CONTROL,
};
use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};

pub(super) fn prepare_private_directory(path: &Path) -> io::Result<()> {
    let user = current_user()?;
    let user_sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut sid_string = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user_sid, &mut sid_string) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut sid_length = 0;
    while unsafe { *sid_string.add(sid_length) } != 0 {
        sid_length += 1;
    }
    let sid =
        String::from_utf16_lossy(unsafe { std::slice::from_raw_parts(sid_string, sid_length) });
    unsafe { LocalFree(sid_string as _) };

    let sddl: Vec<u16> = format!("O:{sid}D:P(A;OICI;FA;;;{sid})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            ptr::null_mut(),
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }

    let result = (|| {
        let absolute = std::path::absolute(path)?;
        let path = match (absolute.parent(), absolute.file_name()) {
            (Some(parent), Some(name)) => {
                std::fs::create_dir_all(parent)?;
                std::fs::canonicalize(parent)?.join(name)
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "private directory must not be a filesystem root",
                ));
            }
        };
        let path_wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        if unsafe { CreateDirectoryW(path_wide.as_ptr(), &attributes) } != 0 {
            return Ok(());
        }
        let err = io::Error::last_os_error();
        if err.kind() != io::ErrorKind::AlreadyExists {
            return Err(err);
        }
        let _guard = validate_private_directory(&path)?;
        Ok(())
    })();

    unsafe { LocalFree(descriptor as _) };
    result
}

pub(super) fn validate_private_socket_path(socket_path: &Path) -> io::Result<(PathBuf, OwnedHandle)> {
    let absolute = std::path::absolute(socket_path)?;
    let directory = absolute.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let socket_name = absolute.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let (directory, guard) = validate_private_directory(directory)?;
    Ok((directory.join(socket_name), guard))
}

fn validate_private_directory(directory: &Path) -> io::Result<(PathBuf, OwnedHandle)> {
    let parent = directory.parent().ok_or(io::ErrorKind::InvalidInput)?;
    let directory_name = directory.file_name().ok_or(io::ErrorKind::InvalidInput)?;
    let directory = std::fs::canonicalize(parent)?.join(directory_name);
    let path_wide: Vec<u16> = directory.as_os_str().encode_wide().chain(Some(0)).collect();

    let handle = unsafe {
        CreateFileW(
            path_wide.as_ptr(),
            READ_CONTROL | FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let guard = unsafe { OwnedHandle::from_raw_handle(handle) };
    let mut info: BY_HANDLE_FILE_INFORMATION = unsafe { std::mem::zeroed() };
    if unsafe { GetFileInformationByHandle(handle, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0
        || info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "untrusted socket directory",
        ));
    }

    let user = current_user()?;
    let user_sid = unsafe { (*(user.as_ptr().cast::<TOKEN_USER>())).User.Sid };
    let mut owner = ptr::null_mut();
    let mut dacl = ptr::null_mut();
    let mut descriptor = ptr::null_mut();
    let status = unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
            &mut owner,
            ptr::null_mut(),
            &mut dacl,
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }

    let trusted = unsafe {
        let mut control = 0;
        let mut revision = 0;
        let mut ace = ptr::null_mut();
        !owner.is_null()
            && EqualSid(owner, user_sid) != 0
            && GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) != 0
            && control & SE_DACL_PROTECTED != 0
            && !dacl.is_null()
            && (*dacl).AceCount == 1
            && GetAce(dacl, 0, &mut ace) != 0
            && (*(ace.cast::<ACE_HEADER>())).AceType == ACCESS_ALLOWED_ACE_TYPE as u8
            && (*(ace.cast::<ACE_HEADER>())).AceFlags
                & (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8
                == (OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE) as u8
            && (*(ace.cast::<ACCESS_ALLOWED_ACE>())).Mask == FILE_ALL_ACCESS
            && EqualSid(
                ptr::addr_of!((*(ace.cast::<ACCESS_ALLOWED_ACE>())).SidStart)
                    .cast_mut()
                    .cast(),
                user_sid,
            ) != 0
    };
    unsafe { LocalFree(descriptor as _) };
    if !trusted {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "untrusted socket directory permissions",
        ));
    }
    Ok((directory, guard))
}

pub(super) fn ensure_non_elevated_peer(socket: RawSocket) -> io::Result<()> {
    let mut pid = 0u32;
    let mut returned = 0;
    if unsafe {
        WSAIoctl(
            socket as _,
            SIO_AF_UNIX_GETPEERPID,
            ptr::null(),
            0,
            ptr::addr_of_mut!(pid).cast(),
            std::mem::size_of_val(&pid) as u32,
            &mut returned,
            ptr::null_mut(),
            None,
        )
    } == SOCKET_ERROR
    {
        return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
    }
    if pid == 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "socket peer PID unavailable",
        ));
    }

    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return Err(io::Error::last_os_error());
    }
    let _process = unsafe { OwnedHandle::from_raw_handle(process) };

    let mut peer_token = ptr::null_mut();
    if unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut peer_token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _peer_token = unsafe { OwnedHandle::from_raw_handle(peer_token) };

    let mut current_token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut current_token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _current_token = unsafe { OwnedHandle::from_raw_handle(current_token) };

    let current_user_buf = token_user(current_token)?;
    let peer_user_buf = token_user(peer_token)?;
    let same_user = unsafe {
        EqualSid(
            (*(current_user_buf.as_ptr().cast::<TOKEN_USER>())).User.Sid,
            (*(peer_user_buf.as_ptr().cast::<TOKEN_USER>())).User.Sid,
        ) != 0
    };

    for token in [current_token, peer_token] {
        let mut elevation: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
        if unsafe {
            GetTokenInformation(
                token,
                TokenElevation,
                ptr::addr_of_mut!(elevation).cast(),
                std::mem::size_of_val(&elevation) as u32,
                &mut returned,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        if !same_user || elevation.TokenIsElevated != 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "implicit daemon connection requires non-elevated current-user tokens",
            ));
        }
    }
    Ok(())
}

fn current_user() -> io::Result<Vec<usize>> {
    let mut token = ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let _token = unsafe { OwnedHandle::from_raw_handle(token) };
    token_user(token)
}

fn token_user(token: HANDLE) -> io::Result<Vec<usize>> {
    let mut length = 0;
    unsafe { GetTokenInformation(token, TokenUser, ptr::null_mut(), 0, &mut length) };
    let mut user = vec![0usize; (length as usize).div_ceil(std::mem::size_of::<usize>())];
    if unsafe {
        GetTokenInformation(
            token,
            TokenUser,
            user.as_mut_ptr().cast(),
            length,
            &mut length,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(user)
}
