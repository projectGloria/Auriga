//! IPC between the Auriga app (normal user) and `auriga-helper.exe`
//! (elevated, on demand). See `proto` for the allow-listed operations.

pub mod frame;
pub mod proto;
#[cfg(windows)]
pub mod win;
