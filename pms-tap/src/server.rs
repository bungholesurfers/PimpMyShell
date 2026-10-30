//! Named-pipe command server running inside explorer.

use std::fs::File;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use std::sync::atomic::Ordering;
use std::time::Duration;

use windows::Win32::Foundation::{ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, GetLastError, HANDLE, HLOCAL, LocalFree};
use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};
use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, FlushFileBuffers, PIPE_ACCESS_DUPLEX};
use windows::Win32::System::Pipes::*;
use windows_core::{HSTRING, w};

use crate::ipc::{PIPE_NAME, read_frame, write_frame};
use crate::{DETACHED, commands, log};

pub fn start() {
    std::thread::spawn(|| {
        if let Err(e) = serve() {
            log(format!("pipe server stopped: {e}"));
        }
    });
}

fn serve() -> windows_core::Result<()> {
    // SYSTEM + the owner (us) only; no Everyone-read default.
    let mut sd = PSECURITY_DESCRIPTOR::default();
    unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(w!("D:P(A;;GA;;;SY)(A;;GA;;;OW)"), SDDL_REVISION_1, &mut sd, None)? };
    let sa = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: sd.0,
        bInheritHandle: false.into(),
    };
    let name = HSTRING::from(PIPE_NAME);
    let result = (|| {
        let mut announced = false;
        let mut busy_retries = 0;
        while !DETACHED.load(Ordering::Acquire) {
            // FIRST_PIPE_INSTANCE: if a previous TAP copy is still serving, refuse rather than share.
            let pipe = unsafe {
                CreateNamedPipeW(
                    &name,
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                    1,
                    1 << 16,
                    1 << 16,
                    0,
                    Some(&sa),
                )
            };
            if pipe.is_invalid() {
                let err = unsafe { GetLastError() };
                if err == ERROR_ACCESS_DENIED && busy_retries < 50 {
                    busy_retries += 1;
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                return Err(windows_core::Error::from(err.to_hresult()));
            }
            // Once: every request gets a fresh instance, and the daemon makes several a second.
            if !announced {
                log("pipe server listening");
                announced = true;
            }
            if let Err(e) = unsafe { ConnectNamedPipe(pipe, None) }
                && unsafe { GetLastError() } != ERROR_PIPE_CONNECTED {
                    log(format!("ConnectNamedPipe: {e}"));
                }
            let mut conn = unsafe { File::from_raw_handle(pipe.0) };
            while let Ok(Some(request)) = read_frame(&mut conn) {
                let response = commands::execute(&request);
                if write_frame(&mut conn, &response).is_err() || DETACHED.load(Ordering::Acquire) {
                    break;
                }
            }
            // Disconnecting discards unread bytes; wait for the client to drain the last response.
            let _ = unsafe { FlushFileBuffers(HANDLE(conn.as_raw_handle())) };
            let _ = unsafe { DisconnectNamedPipe(HANDLE(conn.as_raw_handle())) };
        }
        Ok(())
    })();
    unsafe { LocalFree(Some(HLOCAL(sd.0))) };
    result
}
