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

/// An open, read-only HKLM key. Closed on drop.
pub struct RegKey(windows::Win32::System::Registry::HKEY);

impl Drop for RegKey {
    fn drop(&mut self) {
        unsafe {
            let _ = windows::Win32::System::Registry::RegCloseKey(self.0);
        }
    }
}

impl RegKey {
    /// `Ok(None)` when the key does not exist.
    pub fn open(subkey: &str) -> Result<Option<Self>, String> {
        use windows::Win32::System::Registry::{HKEY, KEY_READ, RegOpenKeyExW};
        let k = wide(subkey);
        let mut h = HKEY::default();
        let rc = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(k.as_ptr()),
                None,
                KEY_READ,
                &mut h,
            )
        };
        match rc {
            ERROR_SUCCESS => Ok(Some(Self(h))),
            e if e.0 == 2 => Ok(None),
            e => Err(format!(
                "RegOpenKeyEx({subkey}) failed: Win32 error {}",
                e.0
            )),
        }
    }

    pub fn subkeys(&self) -> Vec<String> {
        use windows::Win32::System::Registry::RegEnumKeyExW;
        let mut out = Vec::new();
        for i in 0.. {
            let mut buf = [0u16; 256];
            let mut len = buf.len() as u32;
            let rc = unsafe {
                RegEnumKeyExW(
                    self.0,
                    i,
                    Some(windows::core::PWSTR(buf.as_mut_ptr())),
                    &mut len,
                    None,
                    None,
                    None,
                    None,
                )
            };
            if rc != ERROR_SUCCESS {
                break;
            }
            out.push(String::from_utf16_lossy(&buf[..len as usize]));
        }
        out
    }

    /// All string/DWORD values as (name, value-as-text). Other types skipped.
    pub fn values(&self) -> Vec<(String, String)> {
        use windows::Win32::System::Registry::{REG_DWORD, REG_SZ, REG_VALUE_TYPE, RegEnumValueW};
        let mut out = Vec::new();
        for i in 0.. {
            let mut name = [0u16; 512];
            let mut name_len = name.len() as u32;
            let mut data = vec![0u8; 4096];
            let mut data_len = data.len() as u32;
            let mut ty = REG_VALUE_TYPE::default();
            let rc = unsafe {
                RegEnumValueW(
                    self.0,
                    i,
                    Some(windows::core::PWSTR(name.as_mut_ptr())),
                    &mut name_len,
                    None,
                    Some(&mut ty.0),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            if rc.0 == 259 {
                break; // ERROR_NO_MORE_ITEMS
            }
            if rc != ERROR_SUCCESS {
                continue; // e.g. ERROR_MORE_DATA on an oversized binary value
            }
            let n = String::from_utf16_lossy(&name[..name_len as usize]);
            data.truncate(data_len as usize);
            match ty {
                REG_SZ => out.push((n, from_wide(&bytes_to_u16(&data)))),
                REG_DWORD if data.len() >= 4 => out.push((
                    n,
                    u32::from_le_bytes([data[0], data[1], data[2], data[3]]).to_string(),
                )),
                _ => {}
            }
        }
        out
    }
}
