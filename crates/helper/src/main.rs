//! auriga-helper.exe — started on demand (UAC) by the app. Accepts only the
//! typed operations in `helper_ipc::proto::HelperOp`; exits when the app
//! disconnects, on Shutdown, or after IDLE_TIMEOUT_SECS without a request.
#![windows_subsystem = "windows"]

use std::process::ExitCode;

#[cfg(windows)]
fn main() -> ExitCode {
    use helper_ipc::frame::{read_frame, write_frame};
    use helper_ipc::proto::*;
    use helper_ipc::win::{connect_client, is_elevated};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    let args = match HelperArgs::parse(std::env::args().skip(1)) {
        Ok(a) => a,
        Err(_) => return ExitCode::from(2),
    };
    let mut pipe = match connect_client(&args) {
        Ok(p) => p,
        Err(_) => return ExitCode::from(3),
    };
    let hello = HelperMessage::Hello {
        protocol: PROTOCOL_VERSION,
        token: args.token.clone(),
        helper_pid: std::process::id(),
        elevated: is_elevated(),
    };
    if write_frame(&mut pipe, &hello).is_err() {
        return ExitCode::from(4);
    }

    // Idle watchdog.
    let start = Instant::now();
    let last = Arc::new(AtomicU64::new(0));
    {
        let last = last.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                let idle = start.elapsed().as_secs() - last.load(Ordering::Relaxed);
                if idle >= IDLE_TIMEOUT_SECS {
                    std::process::exit(0);
                }
            }
        });
    }

    loop {
        let msg: AppMessage = match read_frame(&mut pipe) {
            Ok(m) => m,
            // App gone or protocol violation: stop.
            Err(_) => return ExitCode::SUCCESS,
        };
        last.store(start.elapsed().as_secs(), Ordering::Relaxed);
        let AppMessage::Request { id, op } = msg;
        let (result, stop) = match op {
            HelperOp::Ping { nonce } => (HelperResult::Pong { nonce }, false),
            HelperOp::Shutdown => (HelperResult::Ok, true),
        };
        if write_frame(&mut pipe, &HelperMessage::Response { id, result }).is_err() || stop {
            return ExitCode::SUCCESS;
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    ExitCode::FAILURE
}
