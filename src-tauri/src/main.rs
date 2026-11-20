// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--dump-snapshot") {
        #[cfg(windows)]
        unsafe {
            // GUI-subsystem exe: reuse the invoking terminal for output.
            use windows::Win32::System::Console::{ATTACH_PARENT_PROCESS, AttachConsole};
            let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        }
        std::process::exit(auriga_lib::dump::dump_snapshot(
            args.get(i + 1).map(String::as_str),
        ));
    }
    auriga_lib::run()
}
