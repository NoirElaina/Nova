//! 跨平台本地 Unix Domain Socket 抽象（Linux `0700`/`0600` 与 Windows SID DACL + 非提权认证）。

#[cfg(windows)]
mod windows_security;

use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const APP_IDENTIFIER: &str = "com.tauri-app.nova";

/// 解析当前数据目录对应的共享守护进程 UDS 路径，确保长度安全落入 `sun_path` 限制。
pub fn default_daemon_socket_path() -> Result<PathBuf, String> {
    if let Some(explicit) = std::env::var_os("NOVA_DAEMON_SOCKET").filter(|v| !v.is_empty()) {
        return Ok(PathBuf::from(explicit));
    }

    let data_dir = match std::env::var_os("NOVA_DATA_DIR").filter(|v| !v.is_empty()) {
        Some(dir) => PathBuf::from(dir),
        None => dirs::data_dir()
            .ok_or_else(|| "无法解析系统数据目录".to_string())?
            .join(APP_IDENTIFIER),
    };

    let preferred = data_dir.join("daemon").join("nova.sock");
    if preferred.as_os_str().len() <= 96 {
        return Ok(preferred);
    }

    // `sun_path` 上限 ~108 字节；过长用户目录改用数据目录哈希映射的短路径，仍受私有权限隔离。
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in data_dir.to_string_lossy().as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    Ok(std::env::temp_dir()
        .join(format!("nova-d-{hash:016x}"))
        .join("n.sock"))
}

/// 准备私有套接字父目录：Linux 严格校验并收敛至 `0700`，Windows 创建当前用户独占保护 DACL。
pub async fn prepare_private_socket_directory(socket_dir: impl AsRef<Path>) -> io::Result<()> {
    platform::prepare_private_socket_directory(socket_dir.as_ref()).await
}

/// 仅校验现有私有套接字路径（Windows 返回目录锁柄；调用方需在连接期间保持存活防篡改）。
#[cfg(windows)]
pub type PathValidationGuard = std::os::windows::io::OwnedHandle;

#[cfg(not(windows))]
pub type PathValidationGuard = ();

pub fn validate_private_socket_path(
    socket_path: impl AsRef<Path>,
) -> io::Result<(PathBuf, PathValidationGuard)> {
    platform::validate_private_socket_path(socket_path.as_ref())
}

/// 异步 Unix Domain Socket 服务端监听器。
pub struct UnixListener {
    inner: platform::Listener,
}

impl UnixListener {
    pub async fn bind(socket_path: impl AsRef<Path>) -> io::Result<Self> {
        platform::bind_listener(socket_path.as_ref())
            .await
            .map(|inner| Self { inner })
    }

    pub async fn accept(&mut self) -> io::Result<UnixStream> {
        self.inner.accept().await.map(|inner| UnixStream { inner })
    }
}

/// 异步 Unix Domain Socket 数据流（实现 `AsyncRead + AsyncWrite + Unpin`）。
pub struct UnixStream {
    inner: platform::Stream,
}

impl UnixStream {
    pub async fn connect(socket_path: impl AsRef<Path>) -> io::Result<Self> {
        platform::connect_stream(socket_path.as_ref())
            .await
            .map(|inner| Self { inner })
    }

    /// 校验对端属于当前用户且双方均未提权（Windows 强制，Unix 由 `0700/0600` 目录与文件权限保证）。
    pub fn ensure_trusted_peer(&self) -> io::Result<()> {
        platform::ensure_trusted_peer(&self.inner)
    }
}

impl AsyncRead for UnixStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_read(cx, buf)
    }
}

impl AsyncWrite for UnixStream {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

#[cfg(unix)]
mod platform {
    use std::io::{self, ErrorKind};
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::path::{Path, PathBuf};

    use tokio::fs;
    use tokio::net::{UnixListener as TokioUnixListener, UnixStream as TokioUnixStream};

    const SOCKET_DIR_MODE: u32 = 0o700;
    const SOCKET_FILE_MODE: u32 = 0o600;

    pub(super) type Stream = TokioUnixStream;

    pub(super) struct Listener(TokioUnixListener);

    pub(super) async fn prepare_private_socket_directory(socket_dir: &Path) -> io::Result<()> {
        let mut builder = fs::DirBuilder::new();
        builder.mode(SOCKET_DIR_MODE);
        builder.recursive(true);
        match builder.create(socket_dir).await {
            Ok(()) => {}
            Err(err) if err.kind() == ErrorKind::AlreadyExists => {}
            Err(err) => return Err(err),
        }

        let metadata = fs::symlink_metadata(socket_dir).await?;
        if !metadata.is_dir() {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!("socket directory path is not a directory: {}", socket_dir.display()),
            ));
        }
        if metadata.uid() != unsafe { libc::geteuid() } {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "socket directory must be owned by current user",
            ));
        }
        if metadata.mode() & 0o777 != SOCKET_DIR_MODE {
            fs::set_permissions(socket_dir, std::fs::Permissions::from_mode(SOCKET_DIR_MODE))
                .await?;
        }
        Ok(())
    }

    pub(super) fn validate_private_socket_path(socket_path: &Path) -> io::Result<(PathBuf, ())> {
        let absolute = std::path::absolute(socket_path)?;
        let dir = absolute.parent().ok_or(ErrorKind::InvalidInput)?;
        let metadata = std::fs::symlink_metadata(dir)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "untrusted socket directory",
            ));
        }
        if metadata.uid() != unsafe { libc::geteuid() } || (metadata.mode() & 0o077 != 0) {
            return Err(io::Error::new(
                ErrorKind::PermissionDenied,
                "socket directory must be mode 0700 owned by current user",
            ));
        }
        if let Ok(sock_meta) = std::fs::symlink_metadata(&absolute) {
            if !sock_meta.file_type().is_socket() || sock_meta.uid() != unsafe { libc::geteuid() }
            {
                return Err(io::Error::new(
                    ErrorKind::PermissionDenied,
                    "untrusted socket entry",
                ));
            }
        }
        Ok((absolute, ()))
    }

    pub(super) async fn bind_listener(socket_path: &Path) -> io::Result<Listener> {
        if let Some(parent) = socket_path.parent() {
            prepare_private_socket_directory(parent).await?;
        }
        if fs::symlink_metadata(socket_path).await.is_ok() {
            let _ = fs::remove_file(socket_path).await;
        }
        let listener = TokioUnixListener::bind(socket_path)?;
        fs::set_permissions(socket_path, std::fs::Permissions::from_mode(SOCKET_FILE_MODE))
            .await?;
        Ok(Listener(listener))
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            self.0.accept().await.map(|(stream, _)| stream)
        }
    }

    pub(super) async fn connect_stream(socket_path: &Path) -> io::Result<Stream> {
        TokioUnixStream::connect(socket_path).await
    }

    pub(super) fn ensure_trusted_peer(_stream: &Stream) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(windows)]
mod platform {
    use std::io;
    use std::net::Shutdown;
    use std::ops::Deref;
    use std::os::windows::io::{AsRawSocket, AsSocket, BorrowedSocket, OwnedHandle};
    use std::path::{Path, PathBuf};
    use std::pin::Pin;
    use std::task::{Context, Poll, ready};

    use async_io::Async;
    use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
    use tokio::task;
    use tokio_util::compat::{Compat, FuturesAsyncReadCompatExt};

    use super::windows_security;

    pub(super) struct Stream(Compat<Async<WindowsUnixStream>>);

    pub(super) struct Listener(Async<WindowsUnixListener>);

    pub(super) async fn prepare_private_socket_directory(socket_dir: &Path) -> io::Result<()> {
        let dir = socket_dir.to_path_buf();
        spawn_blocking_io(move || windows_security::prepare_private_directory(&dir)).await
    }

    pub(super) fn validate_private_socket_path(
        socket_path: &Path,
    ) -> io::Result<(PathBuf, OwnedHandle)> {
        windows_security::validate_private_socket_path(socket_path)
    }

    pub(super) async fn bind_listener(socket_path: &Path) -> io::Result<Listener> {
        if let Some(parent) = socket_path.parent() {
            prepare_private_socket_directory(parent).await?;
        }
        let (validated_path, _guard) = validate_private_socket_path(socket_path)?;
        if tokio::fs::try_exists(&validated_path).await.unwrap_or(false) {
            let _ = tokio::fs::remove_file(&validated_path).await;
        }
        let listener =
            spawn_blocking_io(move || uds_windows::UnixListener::bind(validated_path)).await?;
        Async::new(WindowsUnixListener(listener)).map(Listener)
    }

    impl Listener {
        pub(super) async fn accept(&mut self) -> io::Result<Stream> {
            let (stream, _) = self.0.read_with(|listener| listener.accept()).await?;
            Async::new(WindowsUnixStream(stream))
                .map(FuturesAsyncReadCompatExt::compat)
                .map(Stream)
        }
    }

    pub(super) async fn connect_stream(socket_path: &Path) -> io::Result<Stream> {
        let path = socket_path.to_path_buf();
        let stream = spawn_blocking_io(move || uds_windows::UnixStream::connect(path)).await?;
        Async::new(WindowsUnixStream(stream))
            .map(FuturesAsyncReadCompatExt::compat)
            .map(Stream)
    }

    pub(super) fn ensure_trusted_peer(stream: &Stream) -> io::Result<()> {
        windows_security::ensure_non_elevated_peer(stream.0.get_ref().get_ref().as_raw_socket())
    }

    async fn spawn_blocking_io<T: Send + 'static>(
        operation: impl FnOnce() -> io::Result<T> + Send + 'static,
    ) -> io::Result<T> {
        task::spawn_blocking(operation)
            .await
            .map_err(|err| io::Error::other(format!("blocking socket task failed: {err}")))?
    }

    pub(super) struct WindowsUnixListener(uds_windows::UnixListener);
    pub(super) struct WindowsUnixStream(uds_windows::UnixStream);

    impl Deref for WindowsUnixListener {
        type Target = uds_windows::UnixListener;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl Deref for WindowsUnixStream {
        type Target = uds_windows::UnixStream;
        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl AsSocket for WindowsUnixListener {
        fn as_socket(&self) -> BorrowedSocket<'_> {
            unsafe { BorrowedSocket::borrow_raw(self.as_raw_socket()) }
        }
    }

    impl AsSocket for WindowsUnixStream {
        fn as_socket(&self) -> BorrowedSocket<'_> {
            unsafe { BorrowedSocket::borrow_raw(self.as_raw_socket()) }
        }
    }

    impl io::Read for WindowsUnixStream {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            io::Read::read(&mut self.0, buf)
        }
    }

    impl io::Write for WindowsUnixStream {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            io::Write::write(&mut self.0, buf)
        }

        fn flush(&mut self) -> io::Result<()> {
            io::Write::flush(&mut self.0)
        }
    }

    impl AsyncRead for Stream {
        fn poll_read(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Pin::new(&mut self.get_mut().0).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for Stream {
        fn poll_write(
            self: Pin<&mut Self>,
            cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.get_mut().0).poll_write(cx, buf)
        }

        fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.get_mut().0).poll_flush(cx)
        }

        fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            let stream = &mut self.get_mut().0;
            ready!(Pin::new(&mut *stream).poll_flush(cx))?;
            stream.get_ref().get_ref().shutdown(Shutdown::Write)?;
            Poll::Ready(Ok(()))
        }
    }

    unsafe impl async_io::IoSafe for WindowsUnixListener {}
    unsafe impl async_io::IoSafe for WindowsUnixStream {}
}
