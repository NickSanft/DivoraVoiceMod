//! v1.61.0: DPAPI for the current Windows user — `CryptProtectData` and
//! `CryptUnprotectData` — which `src-tauri` stores the OBS password with
//! (its `obs::files`, D28).
//!
//! crypt32.dll is loaded at run time, so the exe gains no DLL import. The
//! `windows` crate binds both functions through windows-link's raw-dylib
//! imports, so calling either binding would put crypt32.dll in
//! `divora-app.exe`'s load-time import table, which no native module has
//! added to. So, as [`mf`](super::mf) does, it is loaded with
//! `LoadLibraryExW` (System32 only) and the two functions are resolved with
//! `GetProcAddress`, once per process. The blob and prompt types and the
//! flag come from the binding (`Win32_Security_Cryptography`), which imports
//! nothing unless a function is called.
//!
//! No copy is kept: what DPAPI hands back is copied into the returned `Vec`,
//! and its own buffer is zeroed before `LocalFree`. The caller wraps a
//! plaintext `Vec` in `Zeroizing` the moment it returns.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{s, w, BOOL, PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{LocalFree, HLOCAL, HMODULE};
use windows::Win32::Security::Cryptography::{
    CRYPTPROTECT_PROMPTSTRUCT, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

use crate::dpapi::DpapiError;

// The signatures windows-rs 0.61.3 declares for the two exports
// (Win32/Security/Cryptography/mod.rs, `windows_link::link!`).
type FnProtect = unsafe extern "system" fn(
    *const CRYPT_INTEGER_BLOB,
    PCWSTR,
    *const CRYPT_INTEGER_BLOB,
    *const c_void,
    *const CRYPTPROTECT_PROMPTSTRUCT,
    u32,
    *mut CRYPT_INTEGER_BLOB,
) -> BOOL;
type FnUnprotect = unsafe extern "system" fn(
    *const CRYPT_INTEGER_BLOB,
    *mut PWSTR,
    *const CRYPT_INTEGER_BLOB,
    *const c_void,
    *const CRYPTPROTECT_PROMPTSTRUCT,
    u32,
    *mut CRYPT_INTEGER_BLOB,
) -> BOOL;

/// crypt32's two entry points.
struct Crypt32 {
    protect: FnProtect,
    unprotect: FnUnprotect,
}

static CRYPT32: OnceLock<Option<Crypt32>> = OnceLock::new();

/// crypt32, loaded once; the DLL stays loaded for the life of the process,
/// so the pointers can't dangle. `None`: it couldn't be loaded.
fn api() -> Result<&'static Crypt32, DpapiError> {
    CRYPT32
        .get_or_init(load)
        .as_ref()
        .ok_or(DpapiError::Unavailable)
}

fn load() -> Option<Crypt32> {
    // SAFETY: a static, NUL-terminated wide string; the search is limited
    // to System32, so no DLL planted next to the app is loaded.
    let module =
        unsafe { LoadLibraryExW(w!("crypt32.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.ok()?;
    // SAFETY: each name is resolved from crypt32, which exports it, as the
    // signature windows-rs declares for it (above).
    unsafe {
        Some(Crypt32 {
            protect: sym(module, s!("CryptProtectData"))?,
            unprotect: sym(module, s!("CryptUnprotectData"))?,
        })
    }
}

/// `name` from `module` as a function pointer of type `F`.
///
/// SAFETY: `F` must be the export's real signature.
unsafe fn sym<F: Copy>(module: HMODULE, name: PCSTR) -> Option<F> {
    // SAFETY: `module` is loaded and `name` is a static C string.
    let f = unsafe { GetProcAddress(module, name) }?;
    // SAFETY: the caller guarantees `F` is the export's signature; a FARPROC
    // and an `extern "system" fn` pointer have the same size and ABI.
    Some(unsafe { std::mem::transmute_copy::<_, F>(&f) })
}

/// A read-only blob over `bytes` (DPAPI never writes an input).
fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, DpapiError> {
    Ok(CRYPT_INTEGER_BLOB {
        cbData: u32::try_from(bytes.len()).map_err(|_| DpapiError::TooBig)?,
        pbData: bytes.as_ptr().cast_mut(),
    })
}

/// What DPAPI wrote into `out`, copied; its buffer zeroed and freed.
fn take(out: CRYPT_INTEGER_BLOB) -> Vec<u8> {
    if out.pbData.is_null() {
        return Vec::new();
    }
    let len = out.cbData as usize;
    // SAFETY: on success DPAPI hands back `cbData` bytes at `pbData`, which
    // this process owns until `LocalFree`; they are read, then zeroed, then
    // freed once.
    unsafe {
        let copy = std::slice::from_raw_parts(out.pbData, len).to_vec();
        std::ptr::write_bytes(out.pbData, 0, len);
        let _ = LocalFree(Some(HLOCAL(out.pbData.cast())));
        copy
    }
}

/// The last Windows error, as an HRESULT.
fn failed() -> DpapiError {
    DpapiError::Windows(windows::core::Error::from_win32().code().0)
}

/// `plain` encrypted for the current Windows user, with `entropy` (none
/// when empty), and no description or prompt (`CRYPTPROTECT_UI_FORBIDDEN`).
/// The blob opens only for this user on this PC, and only with the same
/// entropy.
pub fn protect(plain: &[u8], entropy: &[u8]) -> Result<Vec<u8>, DpapiError> {
    let api = api()?;
    let input = blob(plain)?;
    let entropy = blob(entropy)?;
    let entropy_ptr: *const CRYPT_INTEGER_BLOB = if entropy.cbData == 0 {
        std::ptr::null()
    } else {
        &raw const entropy
    };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: `input` and `entropy` point at live slices DPAPI only reads,
    // for the length each says; the description, reserved and prompt
    // pointers are null, which DPAPI allows; `out` is written by DPAPI and
    // taken (freed) below only on success.
    let ok = unsafe {
        (api.protect)(
            &raw const input,
            PCWSTR::null(),
            entropy_ptr,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut out,
        )
    };
    if !ok.as_bool() {
        return Err(failed());
    }
    Ok(take(out))
}

/// A blob [`protect`] made, opened with the same `entropy` (none when
/// empty). Fails for another user's blob, another entropy, or one that
/// isn't DPAPI's. The plaintext is the caller's to wipe.
pub fn unprotect(blob_bytes: &[u8], entropy: &[u8]) -> Result<Vec<u8>, DpapiError> {
    let api = api()?;
    let input = blob(blob_bytes)?;
    let entropy = blob(entropy)?;
    let entropy_ptr: *const CRYPT_INTEGER_BLOB = if entropy.cbData == 0 {
        std::ptr::null()
    } else {
        &raw const entropy
    };
    let mut out = CRYPT_INTEGER_BLOB::default();
    // SAFETY: as in `protect`; the description out-pointer is null, so DPAPI
    // allocates none.
    let ok = unsafe {
        (api.unprotect)(
            &raw const input,
            std::ptr::null_mut(),
            entropy_ptr,
            std::ptr::null(),
            std::ptr::null(),
            CRYPTPROTECT_UI_FORBIDDEN,
            &raw mut out,
        )
    };
    if !ok.as_bool() {
        return Err(failed());
    }
    Ok(take(out))
}
