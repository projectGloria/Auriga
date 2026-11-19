//! Spawns the real helper binary *unelevated* over the real pipe transport.
//! The elevated (UAC) path is identical except for the launch call and
//! cannot be exercised non-interactively.
#![cfg(windows)]

use helper_ipc::proto::{HelperOp, HelperResult};
use helper_ipc::win::{HelperSession, LaunchMode};
use std::path::Path;

#[test]
fn ping_roundtrip_and_shutdown() {
    let exe = Path::new(env!("CARGO_BIN_EXE_auriga-helper"));
    let mut s = HelperSession::launch(exe, LaunchMode::Unelevated).expect("launch");
    assert!(s.is_alive());
    for nonce in [1u64, 0xDEAD_BEEF, u64::MAX] {
        assert_eq!(s.request(HelperOp::Ping { nonce }).unwrap(), HelperResult::Pong { nonce });
    }
    assert_eq!(s.shutdown(), Some(0), "helper should exit cleanly");
}

#[test]
fn missing_helper_binary_is_reported() {
    let r = HelperSession::launch(Path::new(r"C:\does\not\exist.exe"), LaunchMode::Elevated);
    assert!(matches!(r, Err(helper_ipc::win::IpcError::HelperMissing(_))));
}
