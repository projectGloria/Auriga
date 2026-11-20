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
                // u64::MAX = a request is running (never idle).
                let idle = start.elapsed().as_secs().saturating_sub(last.load(Ordering::Relaxed));
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
        last.store(u64::MAX, Ordering::Relaxed);
        let AppMessage::Request { id, op } = msg;
        let stop = matches!(op, HelperOp::Shutdown);
        let result = match op.validate() {
            Err(message) => HelperResult::Error { message },
            Ok(()) => dispatch(op),
        };
        last.store(start.elapsed().as_secs(), Ordering::Relaxed);
        if write_frame(&mut pipe, &HelperMessage::Response { id, result }).is_err() || stop {
            return ExitCode::SUCCESS;
        }
    }
}

/// Every write re-reads the device and re-runs the checks in
/// `auriga_core::fix::validate` inside `auriga_core::fix::win`; the request
/// only names the device and the value.
#[cfg(windows)]
fn dispatch(op: helper_ipc::proto::HelperOp) -> helper_ipc::proto::HelperResult {
    use auriga_core::fix::win;
    use helper_ipc::proto::{HelperOp as Op, HelperResult as R};
    let err = |message: String| R::Error { message };
    match op {
        Op::Ping { nonce } => R::Pong { nonce },
        Op::Shutdown => R::Ok,
        Op::SetAdapterProperty {
            instance_id,
            keyword,
            value,
        } => win::set_adapter_property(&instance_id, &keyword, value.as_deref())
            .map_or_else(err, |previous| R::PropertySet { previous }),
        Op::SetPnpCapabilities { instance_id, value } => {
            win::set_pnp_capabilities(&instance_id, value)
                .map_or_else(err, |previous| R::PnpSet { previous })
        }
        Op::RestartDevice { instance_id } => win::restart_device(&instance_id)
            .map_or_else(err, |reboot_required| R::Restarted { reboot_required }),
        Op::CreateRestorePoint { description } => R::RestorePoint {
            outcome: win::create_restore_point(&description),
        },
        Op::RollbackDriver { instance_id } => win::rollback_driver(&instance_id)
            .map_or_else(err, |reboot_required| R::RolledBack { reboot_required }),
        Op::ListMinidumps => {
            let (files, warnings) = win::list_minidumps();
            R::Minidumps { files, warnings }
        }
    }
}

#[cfg(not(windows))]
fn main() -> ExitCode {
    ExitCode::FAILURE
}
