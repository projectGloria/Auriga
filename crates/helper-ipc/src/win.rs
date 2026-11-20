//! Named-pipe transport. The app is the pipe *server*; the helper connects.
//!
//! Hardening:
//! - random pipe name, `FILE_FLAG_FIRST_PIPE_INSTANCE` (no squatting) and a
//!   single instance, `PIPE_REJECT_REMOTE_CLIENTS`;
//! - DACL: only the current user SID and Administrators;
//! - app checks the connected client PID == the PID it launched;
//! - helper checks the server PID == `--parent-pid`, connects with
//!   `SECURITY_IDENTIFICATION` (the app cannot impersonate the elevated
//!   helper), and must echo the one-time token in its Hello.

use crate::frame::{FrameError, read_frame, write_frame};
use crate::proto::*;
use std::io::{Read, Write};
use std::os::windows::io::AsRawHandle;
use std::path::Path;
use std::time::{Duration, Instant};
use windows::Win32::Foundation::*;
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES, TOKEN_ELEVATION, TOKEN_QUERY,
    TOKEN_USER, TokenElevation, TokenUser,
};
use windows::Win32::Storage::FileSystem::*;
use windows::Win32::System::Pipes::*;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, GetExitCodeProcess, GetProcessId, OpenProcessToken,
    WaitForSingleObject,
};
use windows::Win32::UI::Shell::{
    SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
};
use windows::Win32::UI::WindowsAndMessaging::SW_HIDE;
use windows::core::{PCWSTR, PWSTR, w};

#[derive(Debug, thiserror::Error)]
pub enum IpcError {
    #[error("the UAC prompt was declined")]
    UacDeclined,
    #[error("helper executable not found at {0}")]
    HelperMissing(String),
    #[error("helper exited before connecting (exit code {0})")]
    HelperExited(u32),
    #[error("helper did not connect within {0:?}")]
    Timeout(Duration),
    #[error("helper failed verification: {0}")]
    Verification(String),
    #[error("helper is not elevated")]
    NotElevated,
    #[error("protocol error: {0}")]
    Protocol(String),
    #[error("transport: {0}")]
    Frame(#[from] FrameError),
    #[error("windows: {0}")]
    Win(#[from] windows::core::Error),
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct Handle(HANDLE);
// SAFETY: kernel handles may be used from any thread.
unsafe impl Send for Handle {}
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

/// Blocking byte-stream over a pipe handle.
pub struct Pipe(Handle);

impl Read for Pipe {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        match unsafe { ReadFile(self.0.0, Some(buf), Some(&mut n), None) } {
            Ok(()) => Ok(n as usize),
            Err(e) if e.code() == ERROR_BROKEN_PIPE.to_hresult() => Ok(0),
            Err(e) => Err(std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF)),
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut n = 0u32;
        unsafe { WriteFile(self.0.0, Some(buf), Some(&mut n), None) }
            .map_err(|e| std::io::Error::from_raw_os_error(e.code().0 & 0xFFFF))?;
        Ok(n as usize)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------- tokens --

fn current_user_sid_string() -> Result<String, IpcError> {
    unsafe {
        let mut tok = HANDLE::default();
        OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok)?;
        let tok = Handle(tok);
        let mut len = 0u32;
        let _ = GetTokenInformation(tok.0, TokenUser, None, 0, &mut len);
        let mut buf = vec![0u64; (len as usize).div_ceil(8)];
        GetTokenInformation(
            tok.0,
            TokenUser,
            Some(buf.as_mut_ptr().cast()),
            len,
            &mut len,
        )?;
        let user = &*(buf.as_ptr() as *const TOKEN_USER);
        let mut s = PWSTR::null();
        ConvertSidToStringSidW(user.User.Sid, &mut s)?;
        let out = s.to_string().map_err(|e| IpcError::Protocol(e.to_string()));
        let _ = LocalFree(Some(HLOCAL(s.0.cast())));
        out
    }
}

/// Whether the current process token is elevated.
pub fn is_elevated() -> bool {
    unsafe {
        let mut tok = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok).is_err() {
            return false;
        }
        let tok = Handle(tok);
        let mut e = TOKEN_ELEVATION::default();
        let mut len = 0u32;
        GetTokenInformation(
            tok.0,
            TokenElevation,
            Some((&mut e as *mut TOKEN_ELEVATION).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        )
        .is_ok()
            && e.TokenIsElevated != 0
    }
}

// ------------------------------------------------------------ app side --

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// Production: `ShellExecuteExW("runas")` → UAC prompt; helper must
    /// report `elevated: true`.
    Elevated,
    /// Tests only: plain child process, elevation not required.
    Unelevated,
}

enum Proc {
    Child(std::process::Child),
    Shell(Handle),
}

impl Proc {
    fn handle(&self) -> HANDLE {
        match self {
            Proc::Child(c) => HANDLE(c.as_raw_handle()),
            Proc::Shell(h) => h.0,
        }
    }
    fn exit_code(&self) -> Option<u32> {
        let h = self.handle();
        unsafe {
            if WaitForSingleObject(h, 0) != WAIT_OBJECT_0 {
                return None;
            }
            let mut code = 0u32;
            let _ = GetExitCodeProcess(h, &mut code);
            Some(code)
        }
    }
}

/// A connected, verified helper.
pub struct HelperSession {
    pipe: Pipe,
    proc: Proc,
    pid: u32,
    elevated: bool,
    next_id: u64,
}

const CONNECT_TIMEOUT: Duration = Duration::from_secs(20);

impl HelperSession {
    pub fn launch(helper_exe: &Path, mode: LaunchMode) -> Result<Self, IpcError> {
        if !helper_exe.is_file() {
            return Err(IpcError::HelperMissing(helper_exe.display().to_string()));
        }
        let rnd = |n| random_hex(n).map_err(|e| IpcError::Protocol(e.to_string()));
        let args = HelperArgs {
            pipe: format!("{PIPE_PREFIX}{}", rnd(16)?),
            token: rnd(32)?,
            parent_pid: unsafe { GetCurrentProcessId() },
        };

        let pipe = create_server_pipe(&args.pipe)?;
        let proc = spawn(helper_exe, &args, mode)?;
        let pid = unsafe { GetProcessId(proc.handle()) };

        // Wait for the client (non-blocking accept loop so a dead helper or
        // timeout can't hang us).
        let start = Instant::now();
        loop {
            match unsafe { ConnectNamedPipe(pipe.0.0, None) } {
                Ok(()) => break,
                Err(e) if e.code() == ERROR_PIPE_CONNECTED.to_hresult() => break,
                Err(e) if e.code() == ERROR_PIPE_LISTENING.to_hresult() => {}
                Err(e) => return Err(e.into()),
            }
            if let Some(code) = proc.exit_code() {
                return Err(IpcError::HelperExited(code));
            }
            if start.elapsed() > CONNECT_TIMEOUT {
                return Err(IpcError::Timeout(CONNECT_TIMEOUT));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let wait_mode = PIPE_READMODE_BYTE | PIPE_WAIT;
        unsafe { SetNamedPipeHandleState(pipe.0.0, Some(&wait_mode), None, None)? };

        let mut client_pid = 0u32;
        unsafe { GetNamedPipeClientProcessId(pipe.0.0, &mut client_pid)? };
        if client_pid != pid {
            return Err(IpcError::Verification(format!(
                "client pid {client_pid} is not the launched helper {pid}"
            )));
        }

        let mut s = Self {
            pipe,
            proc,
            pid,
            elevated: false,
            next_id: 1,
        };
        match read_frame::<_, HelperMessage>(&mut s.pipe)? {
            HelperMessage::Hello {
                protocol,
                token,
                helper_pid,
                elevated,
            } => {
                if protocol != PROTOCOL_VERSION {
                    return Err(IpcError::Verification(format!("protocol {protocol}")));
                }
                if !token_eq(&token, &args.token) {
                    return Err(IpcError::Verification("token mismatch".into()));
                }
                if helper_pid != pid {
                    return Err(IpcError::Verification("helper pid mismatch".into()));
                }
                if mode == LaunchMode::Elevated && !elevated {
                    return Err(IpcError::NotElevated);
                }
                s.elevated = elevated;
            }
            other => return Err(IpcError::Protocol(format!("expected hello, got {other:?}"))),
        }
        Ok(s)
    }

    pub fn pid(&self) -> u32 {
        self.pid
    }
    pub fn elevated(&self) -> bool {
        self.elevated
    }
    pub fn is_alive(&self) -> bool {
        self.proc.exit_code().is_none()
    }

    pub fn request(&mut self, op: HelperOp) -> Result<HelperResult, IpcError> {
        let id = self.next_id;
        self.next_id += 1;
        write_frame(&mut self.pipe, &AppMessage::Request { id, op })?;
        match read_frame::<_, HelperMessage>(&mut self.pipe)? {
            HelperMessage::Response { id: rid, result } if rid == id => Ok(result),
            other => Err(IpcError::Protocol(format!("unexpected reply {other:?}"))),
        }
    }

    /// Ask the helper to exit and wait briefly for it.
    pub fn shutdown(mut self) -> Option<u32> {
        let _ = self.request(HelperOp::Shutdown);
        unsafe {
            let _ = WaitForSingleObject(self.proc.handle(), 3000);
        }
        self.proc.exit_code()
    }
}

fn create_server_pipe(name: &str) -> Result<Pipe, IpcError> {
    let sddl = format!("D:P(A;;GA;;;{})(A;;GA;;;BA)", current_user_sid_string()?);
    let wsddl = wide(&sddl);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            PCWSTR(wsddl.as_ptr()),
            SDDL_REVISION_1,
            &mut sd,
            None,
        )?;
    }
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let wname = wide(name);
    let h = unsafe {
        CreateNamedPipeW(
            PCWSTR(wname.as_ptr()),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_NOWAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            64 * 1024,
            64 * 1024,
            0,
            Some(&sa),
        )
    };
    unsafe {
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }
    if h.is_invalid() {
        return Err(windows::core::Error::from_thread().into());
    }
    Ok(Pipe(Handle(h)))
}

fn spawn(exe: &Path, args: &HelperArgs, mode: LaunchMode) -> Result<Proc, IpcError> {
    match mode {
        LaunchMode::Unelevated => {
            let child = std::process::Command::new(exe)
                .args(args.to_command_line().split(' '))
                .spawn()
                .map_err(|e| IpcError::Protocol(format!("spawn: {e}")))?;
            Ok(Proc::Child(child))
        }
        LaunchMode::Elevated => {
            let file = wide(&exe.display().to_string());
            let params = wide(&args.to_command_line());
            let mut info = SHELLEXECUTEINFOW {
                cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
                fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC,
                lpVerb: w!("runas"),
                lpFile: PCWSTR(file.as_ptr()),
                lpParameters: PCWSTR(params.as_ptr()),
                nShow: SW_HIDE.0,
                ..Default::default()
            };
            if let Err(e) = unsafe { ShellExecuteExW(&mut info) } {
                if e.code() == ERROR_CANCELLED.to_hresult() {
                    return Err(IpcError::UacDeclined);
                }
                return Err(e.into());
            }
            if info.hProcess.is_invalid() {
                return Err(IpcError::Protocol(
                    "no process handle from ShellExecuteEx".into(),
                ));
            }
            Ok(Proc::Shell(Handle(info.hProcess)))
        }
    }
}

// --------------------------------------------------------- helper side --

/// Connect to the app's pipe and verify the server is our parent.
pub fn connect_client(args: &HelperArgs) -> Result<Pipe, IpcError> {
    let wname = wide(&args.pipe);
    let h = unsafe {
        CreateFileW(
            PCWSTR(wname.as_ptr()),
            (GENERIC_READ | GENERIC_WRITE).0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
        )?
    };
    let pipe = Pipe(Handle(h));
    let mut server_pid = 0u32;
    unsafe { GetNamedPipeServerProcessId(pipe.0.0, &mut server_pid)? };
    if server_pid != args.parent_pid {
        return Err(IpcError::Verification(format!(
            "pipe server pid {server_pid} is not parent {}",
            args.parent_pid
        )));
    }
    Ok(pipe)
}
