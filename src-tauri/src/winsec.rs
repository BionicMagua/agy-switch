//! Windows 凭据管理器 (CredRead/CredWrite) 与 DPAPI 封装。
//! 凭据格式与 language_server.exe (zalando/go-keyring + wincred) 完全一致：
//! CRED_TYPE_GENERIC / CRED_PERSIST_LOCAL_MACHINE / CredentialBlob = 原始 UTF-8 字节。

use std::mem::zeroed;
use std::ptr::{null, null_mut};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, LocalFree, ERROR_NOT_FOUND};
use windows_sys::Win32::Security::Credentials::{
    CredDeleteW, CredFree, CredReadW, CredWriteW, CREDENTIALW, CRED_PERSIST_LOCAL_MACHINE,
    CRED_TYPE_GENERIC,
};
use windows_sys::Win32::Security::Cryptography::{
    CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB,
};
use windows_sys::Win32::Security::{
    GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, TokenElevation,
    TokenIntegrityLevel, TOKEN_ELEVATION, TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows_sys::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

/// Windows 对单条凭据 blob 的上限。
pub const MAX_BLOB: usize = 2560;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn is_elevated() -> Result<bool, String> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(format!("读取进程权限失败 ({})", GetLastError()));
        }
        let mut elevation: TOKEN_ELEVATION = zeroed();
        let mut returned = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        );
        let error = GetLastError();
        CloseHandle(token);
        if ok == 0 {
            return Err(format!("读取进程权限失败 ({error})"));
        }
        Ok(elevation.TokenIsElevated != 0)
    }
}

/// TokenIsElevated=false 也可能是 Low，不能据此认定能使用普通用户的 WebView2 缓存。
pub fn integrity_level() -> Result<u32, String> {
    unsafe {
        let mut token = null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return Err(format!("读取进程完整性级别失败 ({})", GetLastError()));
        }
        let mut needed = 0;
        GetTokenInformation(token, TokenIntegrityLevel, null_mut(), 0, &mut needed);
        // usize 保证 TOKEN_MANDATORY_LABEL 的指针对齐。
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        let ok = GetTokenInformation(
            token,
            TokenIntegrityLevel,
            buffer.as_mut_ptr().cast(),
            needed,
            &mut needed,
        );
        let error = GetLastError();
        CloseHandle(token);
        if ok == 0 {
            return Err(format!("读取进程完整性级别失败 ({error})"));
        }
        let label = &*buffer.as_ptr().cast::<TOKEN_MANDATORY_LABEL>();
        let count = *GetSidSubAuthorityCount(label.Label.Sid);
        if count == 0 {
            return Err("进程完整性 SID 无效".into());
        }
        Ok(*GetSidSubAuthority(label.Label.Sid, u32::from(count - 1)))
    }
}

pub fn show_error(message: &str) {
    unsafe {
        MessageBoxW(
            null_mut(),
            wide(message).as_ptr(),
            wide("AGY Switch 启动错误").as_ptr(),
            MB_OK | MB_ICONERROR,
        );
    }
}

/// 读取凭据，连同最后写入时间（判断凭据管理器和降级文件哪个更新）。
pub fn cred_read_at(target: &str) -> Result<Option<(Vec<u8>, SystemTime)>, String> {
    unsafe {
        let t = wide(target);
        let mut p: *mut CREDENTIALW = null_mut();
        if CredReadW(t.as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) == 0 {
            let e = GetLastError();
            if e == ERROR_NOT_FOUND {
                return Ok(None);
            }
            return Err(format!("CredReadW 失败 (错误码 {e})"));
        }
        let c = &*p;
        let v = if c.CredentialBlob.is_null() || c.CredentialBlobSize == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(c.CredentialBlob, c.CredentialBlobSize as usize).to_vec()
        };
        let ft = (u64::from(c.LastWritten.dwHighDateTime) << 32)
            | u64::from(c.LastWritten.dwLowDateTime);
        // FILETIME：1601-01-01 起的 100ns 计数
        let at = ft
            .checked_sub(116_444_736_000_000_000)
            .map(|t| UNIX_EPOCH + Duration::from_nanos(t.saturating_mul(100)))
            .unwrap_or(UNIX_EPOCH);
        CredFree(p as *const _);
        Ok(Some((v, at)))
    }
}

pub fn cred_write(target: &str, user: &str, blob: &[u8]) -> Result<(), String> {
    if blob.len() > MAX_BLOB {
        return Err(format!(
            "凭据长度 {} 字节，超过 Windows 单条凭据上限 {} 字节",
            blob.len(),
            MAX_BLOB
        ));
    }
    unsafe {
        let mut t = wide(target);
        let mut u = wide(user);
        let mut cred: CREDENTIALW = zeroed();
        cred.Type = CRED_TYPE_GENERIC;
        cred.TargetName = t.as_mut_ptr();
        cred.UserName = u.as_mut_ptr();
        cred.CredentialBlobSize = blob.len() as u32;
        cred.CredentialBlob = blob.as_ptr() as *mut u8;
        cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
        if CredWriteW(&cred, 0) == 0 {
            return Err(format!("CredWriteW 失败 (错误码 {})", GetLastError()));
        }
        Ok(())
    }
}

pub fn cred_delete(target: &str) -> Result<(), String> {
    unsafe {
        let t = wide(target);
        if CredDeleteW(t.as_ptr(), CRED_TYPE_GENERIC, 0) == 0 {
            let e = GetLastError();
            if e == ERROR_NOT_FOUND {
                return Ok(());
            }
            return Err(format!("CredDeleteW 失败 (错误码 {e})"));
        }
        Ok(())
    }
}

pub fn protect(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out: CRYPT_INTEGER_BLOB = zeroed();
        if CryptProtectData(&input, null(), null(), null(), null(), 0, &mut out) == 0 {
            return Err(format!("CryptProtectData 失败 (错误码 {})", GetLastError()));
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as *mut _);
        Ok(v)
    }
}

pub fn unprotect(data: &[u8]) -> Result<Vec<u8>, String> {
    unsafe {
        let input = CRYPT_INTEGER_BLOB {
            cbData: data.len() as u32,
            pbData: data.as_ptr() as *mut u8,
        };
        let mut out: CRYPT_INTEGER_BLOB = zeroed();
        if CryptUnprotectData(&input, null_mut(), null(), null(), null(), 0, &mut out) == 0 {
            return Err(format!(
                "CryptUnprotectData 失败 (错误码 {})",
                GetLastError()
            ));
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as *mut _);
        Ok(v)
    }
}
