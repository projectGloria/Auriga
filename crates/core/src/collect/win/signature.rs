//! Driver package signature check, fully offline.
//!
//! 1. Hash the installed INF (`%windir%\INF\oemNN.inf`) and look it up in the
//!    system catalog database (CryptCATAdmin) to find the signing catalog.
//! 2. Verify the INF as a catalog member with the driver policy
//!    (`DRIVER_ACTION_VERIFY`) and read "signed by" from `DRIVER_VER_INFO`.
//! 3. If driver policy rejects it, fall back to Authenticode policy on the
//!    catalog so cross-signed/test-signed packages are classified, not hidden.
//!
//! Revocation checks are disabled and URL retrieval is cache-only, so this
//! never touches the network.

use super::util::{from_wide, wide};
use crate::snapshot::{SignatureInfo, SignatureStatus};
use std::collections::HashMap;
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, HWND};
use windows::Win32::Security::Cryptography::Catalog::*;
use windows::Win32::Security::Cryptography::{
    CERT_NAME_SIMPLE_DISPLAY_TYPE, CertFreeCertificateContext, CertGetNameStringW,
};
use windows::Win32::Security::WinTrust::*;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ, OPEN_EXISTING,
};
use windows::core::{PCWSTR, w};

const CERT_E_UNTRUSTEDROOT: i32 = 0x800B_0109_u32 as i32;
const CERT_E_UNTRUSTEDTESTROOT: i32 = 0x800B_010D_u32 as i32;
const TRUST_E_NOSIGNATURE: i32 = 0x800B_0100_u32 as i32;

pub struct SignatureChecker {
    /// One CATAdmin context per hash algorithm we try (SHA256, then SHA1 for
    /// older catalogs).
    admins: Vec<isize>,
    init_error: Option<String>,
    cache: HashMap<String, SignatureInfo>,
    inf_dir: String,
}

impl SignatureChecker {
    pub fn new() -> Self {
        let mut admins = Vec::new();
        let mut init_error = None;
        for alg in [w!("SHA256"), w!("SHA1")] {
            let mut h: isize = 0;
            match unsafe { CryptCATAdminAcquireContext2(&mut h, None, alg, None, None) } {
                Ok(()) => admins.push(h),
                Err(e) => init_error = Some(format!("CryptCATAdminAcquireContext2 failed: {e}")),
            }
        }
        if !admins.is_empty() {
            init_error = None;
        }
        let windir = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        Self {
            admins,
            init_error,
            cache: HashMap::new(),
            inf_dir: format!(r"{windir}\INF"),
        }
    }

    pub fn init_error(&self) -> Option<&str> {
        self.init_error.as_deref()
    }

    pub fn check_inf(&mut self, inf_name: &str) -> SignatureInfo {
        let key = inf_name.to_ascii_lowercase();
        if let Some(c) = self.cache.get(&key) {
            return c.clone();
        }
        // Only plain file names are accepted (they come from the PnP manager,
        // but never let a path component escape the INF directory).
        let info = if inf_name.contains(['\\', '/', ':']) {
            SignatureInfo {
                detail: Some("unexpected INF path".into()),
                ..Default::default()
            }
        } else {
            let path = format!(r"{}\{}", self.inf_dir, inf_name);
            self.check_path(&path)
        };
        self.cache.insert(key, info.clone());
        info
    }

    fn check_path(&self, inf_path: &str) -> SignatureInfo {
        let wpath = wide(inf_path);
        let file = match unsafe {
            CreateFileW(
                PCWSTR(wpath.as_ptr()),
                GENERIC_READ.0,
                FILE_SHARE_READ,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        } {
            Ok(h) => OwnedHandle(h),
            Err(e) => {
                return SignatureInfo {
                    detail: Some(format!("cannot open INF: {e}")),
                    ..Default::default()
                };
            }
        };

        let mut last_detail = None;
        for &admin in &self.admins {
            let Some(hash) = file_hash(admin, file.0) else {
                continue;
            };
            let cat_info = unsafe { CryptCATAdminEnumCatalogFromHash(admin, &hash, None, None) };
            if cat_info == 0 {
                last_detail = Some("no catalog contains this INF".to_string());
                continue;
            }
            let mut ci = CATALOG_INFO {
                cbStruct: size_of::<CATALOG_INFO>() as u32,
                ..Default::default()
            };
            let ok = unsafe { CryptCATCatalogInfoFromContext(cat_info, &mut ci, 0) }.is_ok();
            unsafe {
                let _ = CryptCATAdminReleaseCatalogContext(admin, cat_info, 0);
            }
            if !ok {
                last_detail = Some("catalog lookup failed".to_string());
                continue;
            }
            let catalog = from_wide(&ci.wszCatalogFile);
            return verify_member(admin, &catalog, inf_path, file.0, &hash);
        }
        SignatureInfo {
            status: if last_detail.is_some() {
                SignatureStatus::NotSigned
            } else {
                SignatureStatus::Unknown
            },
            detail: last_detail.or(Some("could not hash INF".into())),
            ..Default::default()
        }
    }
}

impl Drop for SignatureChecker {
    fn drop(&mut self) {
        for &a in &self.admins {
            unsafe {
                let _ = CryptCATAdminReleaseContext(a, 0);
            }
        }
    }
}

struct OwnedHandle(HANDLE);
impl Drop for OwnedHandle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

fn file_hash(admin: isize, file: HANDLE) -> Option<Vec<u8>> {
    let mut len = 0u32;
    unsafe {
        let _ = CryptCATAdminCalcHashFromFileHandle2(admin, file, &mut len, None, None);
    }
    if len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    unsafe {
        CryptCATAdminCalcHashFromFileHandle2(admin, file, &mut len, Some(buf.as_mut_ptr()), None)
    }
    .ok()?;
    buf.truncate(len as usize);
    Some(buf)
}

fn verify_member(
    admin: isize,
    catalog: &str,
    member_path: &str,
    member: HANDLE,
    hash: &[u8],
) -> SignatureInfo {
    let tag: String = hash.iter().map(|b| format!("{b:02X}")).collect();
    let wcat = wide(catalog);
    let wtag = wide(&tag);
    let wmember = wide(member_path);
    let mut hash_buf = hash.to_vec();

    // --- Driver policy ---
    let mut cat = WINTRUST_CATALOG_INFO {
        cbStruct: size_of::<WINTRUST_CATALOG_INFO>() as u32,
        pcwszCatalogFilePath: PCWSTR(wcat.as_ptr()),
        pcwszMemberTag: PCWSTR(wtag.as_ptr()),
        pcwszMemberFilePath: PCWSTR(wmember.as_ptr()),
        hMemberFile: member,
        pbCalculatedFileHash: hash_buf.as_mut_ptr(),
        cbCalculatedFileHash: hash_buf.len() as u32,
        hCatAdmin: admin,
        ..Default::default()
    };
    let mut ver = DRIVER_VER_INFO {
        cbStruct: size_of::<DRIVER_VER_INFO>() as u32,
        ..Default::default()
    };
    let mut data = base_trust_data();
    data.pPolicyCallbackData = (&mut ver as *mut DRIVER_VER_INFO).cast();
    data.dwUnionChoice = WTD_CHOICE_CATALOG;
    data.Anonymous.pCatalog = &mut cat;
    let mut action = DRIVER_ACTION_VERIFY;
    let rc = run_trust(&mut action, &mut data);
    if !ver.pcSignerCertContext.is_null() {
        unsafe {
            let _ = CertFreeCertificateContext(Some(ver.pcSignerCertContext));
        }
    }
    if rc == 0 {
        let signer = from_wide(&ver.wszSignedBy);
        return SignatureInfo {
            status: SignatureStatus::Valid,
            signer: (!signer.is_empty()).then_some(signer),
            catalog: Some(catalog.to_string()),
            detail: None,
        };
    }

    // --- Authenticode policy on the catalog file itself ---
    let mut file = WINTRUST_FILE_INFO {
        cbStruct: size_of::<WINTRUST_FILE_INFO>() as u32,
        pcwszFilePath: PCWSTR(wcat.as_ptr()),
        ..Default::default()
    };
    let mut data = base_trust_data();
    data.dwUnionChoice = WTD_CHOICE_FILE;
    data.Anonymous.pFile = &mut file;
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    let mut action = WINTRUST_ACTION_GENERIC_VERIFY_V2;
    let rc2 = unsafe {
        WinVerifyTrust(
            HWND::default(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        )
    };
    let signer = leaf_signer(&data);
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(
            HWND::default(),
            &mut action,
            (&mut data as *mut WINTRUST_DATA).cast(),
        );
    }

    let status = match rc2 {
        0 => SignatureStatus::Valid,
        CERT_E_UNTRUSTEDROOT | CERT_E_UNTRUSTEDTESTROOT => SignatureStatus::TestSigned,
        TRUST_E_NOSIGNATURE => SignatureStatus::NotSigned,
        _ => SignatureStatus::Invalid,
    };
    SignatureInfo {
        status,
        signer,
        catalog: Some(catalog.to_string()),
        detail: Some(format!(
            "driver policy: 0x{:08X}; Authenticode: 0x{:08X}",
            rc as u32, rc2 as u32
        )),
    }
}

fn base_trust_data() -> WINTRUST_DATA {
    WINTRUST_DATA {
        cbStruct: size_of::<WINTRUST_DATA>() as u32,
        dwUIChoice: WTD_UI_NONE,
        fdwRevocationChecks: WTD_REVOKE_NONE,
        dwProvFlags: WTD_CACHE_ONLY_URL_RETRIEVAL,
        ..Default::default()
    }
}

/// Verify with state, then close it. Returns the WinVerifyTrust status.
fn run_trust(action: &mut windows::core::GUID, data: &mut WINTRUST_DATA) -> i32 {
    data.dwStateAction = WTD_STATEACTION_VERIFY;
    let rc =
        unsafe { WinVerifyTrust(HWND::default(), action, (data as *mut WINTRUST_DATA).cast()) };
    data.dwStateAction = WTD_STATEACTION_CLOSE;
    unsafe {
        WinVerifyTrust(HWND::default(), action, (data as *mut WINTRUST_DATA).cast());
    }
    rc
}

fn leaf_signer(data: &WINTRUST_DATA) -> Option<String> {
    unsafe {
        if data.hWVTStateData.is_invalid() {
            return None;
        }
        let prov = WTHelperProvDataFromStateData(data.hWVTStateData);
        if prov.is_null() {
            return None;
        }
        let sgnr = WTHelperGetProvSignerFromChain(prov, 0, false, 0);
        if sgnr.is_null() || (*sgnr).csCertChain == 0 || (*sgnr).pasCertChain.is_null() {
            return None;
        }
        let cert = (*(*sgnr).pasCertChain).pCert;
        if cert.is_null() {
            return None;
        }
        let mut buf = [0u16; 256];
        let n = CertGetNameStringW(cert, CERT_NAME_SIMPLE_DISPLAY_TYPE, 0, None, Some(&mut buf));
        (n > 1).then(|| from_wide(&buf))
    }
}
