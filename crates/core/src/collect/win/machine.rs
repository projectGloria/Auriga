use super::registry::{read_dword, read_string};
use crate::snapshot::{MachineInfo, SourceWarning};

const NT_CV: &str = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
const BIOS: &str = r"HARDWARE\DESCRIPTION\System\BIOS";

pub fn collect(warnings: &mut Vec<SourceWarning>) -> MachineInfo {
    let mut warn = |e: String| {
        warnings.push(SourceWarning {
            source: "machine".into(),
            message: e,
            instance_id: None,
        })
    };
    let mut get = |k: &str, v: &str| {
        read_string(k, v).unwrap_or_else(|e| {
            warn(e);
            None
        })
    };
    let build = get(NT_CV, "CurrentBuild");
    let display = get(NT_CV, "DisplayVersion");
    let manufacturer = get(BIOS, "SystemManufacturer");
    let model = get(BIOS, "SystemProductName");
    let ubr = read_dword(NT_CV, "UBR").ok().flatten();
    MachineInfo {
        os_build: build.map(|b| match ubr {
            Some(u) => format!("{b}.{u}"),
            None => b,
        }),
        os_display_version: display,
        manufacturer,
        model,
    }
}
