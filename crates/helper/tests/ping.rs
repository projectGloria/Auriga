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
        assert_eq!(
            s.request(HelperOp::Ping { nonce }).unwrap(),
            HelperResult::Pong { nonce }
        );
    }
    assert_eq!(s.shutdown(), Some(0), "helper should exit cleanly");
}

#[test]
fn missing_helper_binary_is_reported() {
    let r = HelperSession::launch(Path::new(r"C:\does\not\exist.exe"), LaunchMode::Elevated);
    assert!(matches!(
        r,
        Err(helper_ipc::win::IpcError::HelperMissing(_))
    ));
}

/// Unelevated, so nothing can be written even if a check were missing; the
/// point is that each request is refused by validation before any write.
#[test]
fn bad_requests_are_refused_and_helper_keeps_serving() {
    let exe = Path::new(env!("CARGO_BIN_EXE_auriga-helper"));
    let mut s = HelperSession::launch(exe, LaunchMode::Unelevated).expect("launch");
    let refused = |r: HelperResult| matches!(r, HelperResult::Error { .. });
    let ops = [
        // Shell metacharacters / path tricks in names.
        HelperOp::SetAdapterProperty {
            instance_id: r"PCI\X & calc.exe".into(),
            keyword: "MIMOPowerSaveMode".into(),
            value: Some("3".into()),
        },
        HelperOp::SetAdapterProperty {
            instance_id: r"PCI\VEN_8086&DEV_0000\0".into(),
            keyword: r"..\..\Services\x".into(),
            value: Some("3".into()),
        },
        // Device that does not exist.
        HelperOp::SetAdapterProperty {
            instance_id: r"PCI\VEN_FFFF&DEV_FFFF\AURIGA_TEST".into(),
            keyword: "MIMOPowerSaveMode".into(),
            value: Some("3".into()),
        },
        // Exists everywhere, but is not a network adapter.
        HelperOp::SetPnpCapabilities {
            instance_id: r"HTREE\ROOT\0".into(),
            value: Some(8),
        },
        HelperOp::RestartDevice {
            instance_id: r"HTREE\ROOT\0".into(),
        },
        HelperOp::CreateRestorePoint {
            description: "x\u{0}y".into(),
        },
    ];
    for op in ops {
        let r = s.request(op.clone()).unwrap();
        assert!(refused(r.clone()), "{op:?} -> {r:?}");
    }
    assert_eq!(
        s.request(HelperOp::Ping { nonce: 9 }).unwrap(),
        HelperResult::Pong { nonce: 9 }
    );
    assert_eq!(s.shutdown(), Some(0));
}
