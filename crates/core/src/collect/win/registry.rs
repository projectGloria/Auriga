//! Minimal read-only registry access (HKLM). Writes never happen here.

use super::util::{bytes_to_u16, from_wide, wide};
use windows::Win32::Foundation::ERROR_SUCCESS;
use windows::Win32::System::Registry::{
    HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::PCWSTR;

pub const CLASS_ROOT: &str = r"SYSTEM\CurrentControlSet\Control\Class";

pub fn read_dword(subkey: &str, value: &str) -> Result<Option<u32>, String> {
    let k = wide(subkey);
    let v = wide(value);
    let mut data: u32 = 0;
    let mut size = 4u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut data as *mut u32).cast()),
            Some(&mut size),
        )
    };
    match rc {
        ERROR_SUCCESS => Ok(Some(data)),
        e if e.0 == 2 => Ok(None), // ERROR_FILE_NOT_FOUND: value absent
        e => Err(format!(
            "RegGetValue({subkey}\\{value}) failed: Win32 error {}",
            e.0
        )),
    }
}

pub fn read_string(subkey: &str, value: &str) -> Result<Option<String>, String> {
    let k = wide(subkey);
    let v = wide(value);
    let mut size = 0u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            None,
            Some(&mut size),
        )
    };
    if rc.0 == 2 {
        return Ok(None);
    }
    if rc != ERROR_SUCCESS {
        return Err(format!(
            "RegGetValue({subkey}\\{value}) failed: Win32 error {}",
            rc.0
        ));
    }
    let mut buf = vec![0u8; size as usize];
    let rc = unsafe {
        RegGetValueW(
            HKEY_LOCAL_MACHINE,
            PCWSTR(k.as_ptr()),
            PCWSTR(v.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if rc != ERROR_SUCCESS {
        return Err(format!(
            "RegGetValue({subkey}\\{value}) failed: Win32 error {}",
            rc.0
        ));
    }
    Ok(Some(from_wide(&bytes_to_u16(&buf[..size as usize]))))
}
