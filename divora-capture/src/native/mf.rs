//! Media Foundation, loaded at run time.
//!
//! The `windows` crate binds Media Foundation's free functions (`MFStartup`,
//! `MFCreateSinkWriterFromURL`, …) through windows-link's raw-dylib imports:
//! calling any of them puts mfplat.dll / mfreadwrite.dll into
//! `divora-app.exe`'s import table, and Windows N editions — which ship
//! without Media Foundation until the Media Feature Pack is installed —
//! would then refuse to *start the app at all*. So those DLLs are loaded
//! here with `LoadLibraryExW` (System32 only) and every free function is
//! resolved with `GetProcAddress`; a missing DLL becomes
//! [`CaptureError::Unavailable`] with the fix, never a launch failure.
//!
//! COM *methods* (`IMFSinkWriter::WriteSample`, …) are vtable calls and
//! import nothing, so they are used through the `windows` crate as usual.
//! The GUIDs are constants and import nothing either.

use std::ffi::c_void;
use std::sync::OnceLock;

use windows::core::{s, w, Interface, BOOL, GUID, HRESULT, PCSTR, PCWSTR};
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D11::ID3D11Texture2D;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFAttributes, IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaType, IMFSample,
    IMFSinkWriter, MFSTARTUP_FULL, MFT_REGISTER_TYPE_INFO, MF_VERSION,
};
use windows::Win32::System::Com::CoTaskMemFree;
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32,
};

use crate::error::{CaptureError, Context};

/// What the user is told when Media Foundation isn't there.
pub const FEATURE_PACK_HINT: &str = "Screen recording needs Windows' media components. \
     Install the Media Feature Pack (Settings › Apps › Optional features), then restart.";

type FnStartup = unsafe extern "system" fn(u32, u32) -> HRESULT;
type FnShutdown = unsafe extern "system" fn() -> HRESULT;
type FnCreateAttributes = unsafe extern "system" fn(*mut *mut c_void, u32) -> HRESULT;
type FnCreateOut = unsafe extern "system" fn(*mut *mut c_void) -> HRESULT;
type FnCreateMemoryBuffer = unsafe extern "system" fn(u32, *mut *mut c_void) -> HRESULT;
type FnCreateDxgiDeviceManager = unsafe extern "system" fn(*mut u32, *mut *mut c_void) -> HRESULT;
type FnCreateDxgiSurfaceBuffer =
    unsafe extern "system" fn(*const GUID, *mut c_void, u32, BOOL, *mut *mut c_void) -> HRESULT;
type FnTEnumEx = unsafe extern "system" fn(
    GUID,
    u32,
    *const MFT_REGISTER_TYPE_INFO,
    *const MFT_REGISTER_TYPE_INFO,
    *mut *mut *mut c_void,
    *mut u32,
) -> HRESULT;
type FnCreateSinkWriterFromUrl =
    unsafe extern "system" fn(PCWSTR, *mut c_void, *mut c_void, *mut *mut c_void) -> HRESULT;

/// The Media Foundation entry points this crate uses.
pub struct Mf {
    startup: FnStartup,
    shutdown: FnShutdown,
    create_attributes: FnCreateAttributes,
    create_media_type: FnCreateOut,
    create_sample: FnCreateOut,
    create_memory_buffer: FnCreateMemoryBuffer,
    create_dxgi_device_manager: FnCreateDxgiDeviceManager,
    create_dxgi_surface_buffer: FnCreateDxgiSurfaceBuffer,
    tenum_ex: FnTEnumEx,
    create_sink_writer_from_url: FnCreateSinkWriterFromUrl,
}

static MF: OnceLock<Result<Mf, String>> = OnceLock::new();

/// Media Foundation, or why it can't be used. Loaded once per process; the
/// DLLs stay loaded for the life of the process (they are never freed, so
/// the function pointers can't dangle).
pub fn api() -> Result<&'static Mf, CaptureError> {
    MF.get_or_init(load)
        .as_ref()
        .map_err(|why| CaptureError::Unavailable(why.clone()))
}

/// Set to anything to make this process behave as if Media Foundation were
/// missing — how a Windows N machine without the Media Feature Pack is
/// checked on one that has it (`docs/MANUAL_TESTS.md`).
pub const PRETEND_MISSING_ENV: &str = "DIVORA_PRETEND_NO_MEDIA_FOUNDATION";

fn load() -> Result<Mf, String> {
    if std::env::var_os(PRETEND_MISSING_ENV).is_some() {
        tracing::info!("media foundation treated as missing ({PRETEND_MISSING_ENV})");
        return Err(FEATURE_PACK_HINT.to_string());
    }
    let plat = load_system_dll(w!("mfplat.dll"))?;
    let rw = load_system_dll(w!("mfreadwrite.dll"))?;
    // SAFETY (all `sym` calls): each name is resolved from the DLL that
    // exports it, and transmuted to the signature windows-rs 0.61.3 itself
    // declares for that export (Win32/Media/MediaFoundation/mod.rs,
    // `windows_link::link!`). Both DLLs stay loaded for the process's life.
    unsafe {
        Ok(Mf {
            startup: sym(plat, s!("MFStartup"))?,
            shutdown: sym(plat, s!("MFShutdown"))?,
            create_attributes: sym(plat, s!("MFCreateAttributes"))?,
            create_media_type: sym(plat, s!("MFCreateMediaType"))?,
            create_sample: sym(plat, s!("MFCreateSample"))?,
            create_memory_buffer: sym(plat, s!("MFCreateMemoryBuffer"))?,
            create_dxgi_device_manager: sym(plat, s!("MFCreateDXGIDeviceManager"))?,
            create_dxgi_surface_buffer: sym(plat, s!("MFCreateDXGISurfaceBuffer"))?,
            tenum_ex: sym(plat, s!("MFTEnumEx"))?,
            create_sink_writer_from_url: sym(rw, s!("MFCreateSinkWriterFromURL"))?,
        })
    }
}

fn load_system_dll(name: PCWSTR) -> Result<HMODULE, String> {
    // SAFETY: `name` is a static, NUL-terminated wide string; the search is
    // restricted to System32, so no DLL can be planted next to the app.
    unsafe { LoadLibraryExW(name, None, LOAD_LIBRARY_SEARCH_SYSTEM32) }.map_err(|e| {
        tracing::info!(error = %e, "media foundation unavailable");
        FEATURE_PACK_HINT.to_string()
    })
}

/// Resolve `name` from `module` as a function pointer of type `F`.
///
/// SAFETY: `F` must be the export's real signature.
unsafe fn sym<F: Copy>(module: HMODULE, name: PCSTR) -> Result<F, String> {
    debug_assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<usize>());
    // SAFETY: `module` is a loaded module and `name` a static C string.
    let Some(f) = (unsafe { GetProcAddress(module, name) }) else {
        return Err(FEATURE_PACK_HINT.to_string());
    };
    // SAFETY: the caller guarantees `F` is the export's signature; a FARPROC
    // and an `extern "system" fn` pointer have the same size and ABI.
    Ok(unsafe { std::mem::transmute_copy::<_, F>(&f) })
}

/// Take ownership of a COM pointer an `Mf*` call wrote.
///
/// SAFETY: `p` must be null or an owned (`AddRef`'d) pointer to `T`.
unsafe fn owned<T: Interface>(p: *mut c_void, what: &'static str) -> Result<T, CaptureError> {
    if p.is_null() {
        return Err(CaptureError::Windows {
            context: what,
            source: windows::core::Error::from_hresult(windows::Win32::Foundation::E_POINTER),
        });
    }
    // SAFETY: non-null, and owned per the caller's contract.
    Ok(unsafe { T::from_raw(p) })
}

impl Mf {
    /// Start Media Foundation for the life of the returned guard.
    pub fn session(&'static self) -> Result<MfSession, CaptureError> {
        // SAFETY: plain call with constants; balanced by `MfSession::drop`.
        unsafe { (self.startup)(MF_VERSION, MFSTARTUP_FULL) }
            .ok()
            .context("MFStartup")?;
        Ok(MfSession { mf: self })
    }

    pub fn create_attributes(&self, initial: u32) -> Result<IMFAttributes, CaptureError> {
        let mut p = std::ptr::null_mut();
        // SAFETY: `p` is a valid out-pointer; on success it owns the object.
        unsafe { (self.create_attributes)(&raw mut p, initial) }
            .ok()
            .context("MFCreateAttributes")?;
        // SAFETY: written by MFCreateAttributes, owned.
        unsafe { owned(p, "MFCreateAttributes") }
    }

    pub fn create_media_type(&self) -> Result<IMFMediaType, CaptureError> {
        let mut p = std::ptr::null_mut();
        // SAFETY: as above.
        unsafe { (self.create_media_type)(&raw mut p) }
            .ok()
            .context("MFCreateMediaType")?;
        // SAFETY: as above.
        unsafe { owned(p, "MFCreateMediaType") }
    }

    pub fn create_sample(&self) -> Result<IMFSample, CaptureError> {
        let mut p = std::ptr::null_mut();
        // SAFETY: as above.
        unsafe { (self.create_sample)(&raw mut p) }
            .ok()
            .context("MFCreateSample")?;
        // SAFETY: as above.
        unsafe { owned(p, "MFCreateSample") }
    }

    pub fn create_memory_buffer(&self, bytes: u32) -> Result<IMFMediaBuffer, CaptureError> {
        let mut p = std::ptr::null_mut();
        // SAFETY: as above.
        unsafe { (self.create_memory_buffer)(bytes, &raw mut p) }
            .ok()
            .context("MFCreateMemoryBuffer")?;
        // SAFETY: as above.
        unsafe { owned(p, "MFCreateMemoryBuffer") }
    }

    /// A DXGI device manager and its reset token.
    pub fn create_dxgi_device_manager(&self) -> Result<(u32, IMFDXGIDeviceManager), CaptureError> {
        let mut token = 0u32;
        let mut p = std::ptr::null_mut();
        // SAFETY: both are valid out-pointers.
        unsafe { (self.create_dxgi_device_manager)(&raw mut token, &raw mut p) }
            .ok()
            .context("MFCreateDXGIDeviceManager")?;
        // SAFETY: as above.
        Ok((token, unsafe { owned(p, "MFCreateDXGIDeviceManager")? }))
    }

    /// A media buffer over a Direct3D 11 texture (no copy).
    pub fn create_dxgi_surface_buffer(
        &self,
        texture: &ID3D11Texture2D,
    ) -> Result<IMFMediaBuffer, CaptureError> {
        let mut p = std::ptr::null_mut();
        // SAFETY: the texture is alive for the call and the buffer AddRefs
        // it; the IID matches the interface pointer passed.
        unsafe {
            (self.create_dxgi_surface_buffer)(
                &ID3D11Texture2D::IID,
                texture.as_raw(),
                0,
                BOOL(0),
                &raw mut p,
            )
        }
        .ok()
        .context("MFCreateDXGISurfaceBuffer")?;
        // SAFETY: as above.
        unsafe { owned(p, "MFCreateDXGISurfaceBuffer") }
    }

    /// The MFTs registered for `category` matching the given types.
    pub fn tenum_ex(
        &self,
        category: GUID,
        flags: u32,
        input: Option<&MFT_REGISTER_TYPE_INFO>,
        output: Option<&MFT_REGISTER_TYPE_INFO>,
    ) -> Result<Vec<IMFActivate>, CaptureError> {
        let mut array: *mut *mut c_void = std::ptr::null_mut();
        let mut count = 0u32;
        let input = input.map_or(std::ptr::null(), std::ptr::from_ref);
        let output = output.map_or(std::ptr::null(), std::ptr::from_ref);
        // SAFETY: valid out-pointers; the type-info pointers are null or
        // point at live values for the duration of the call.
        unsafe {
            (self.tenum_ex)(
                category,
                flags,
                input,
                output,
                &raw mut array,
                &raw mut count,
            )
        }
        .ok()
        .context("MFTEnumEx")?;
        let mut out = Vec::with_capacity(count as usize);
        if !array.is_null() {
            for i in 0..count as usize {
                // SAFETY: MFTEnumEx returns `count` owned IMFActivate
                // pointers in a CoTaskMemAlloc'd array.
                let p = unsafe { *array.add(i) };
                if !p.is_null() {
                    // SAFETY: owned pointer, per the above.
                    out.push(unsafe { IMFActivate::from_raw(p) });
                }
            }
            // SAFETY: the array (not its elements, now owned by `out`) was
            // allocated with CoTaskMemAlloc by MFTEnumEx.
            unsafe { CoTaskMemFree(Some(array.cast_const().cast())) };
        }
        Ok(out)
    }

    /// A sink writer for the file at `path`, configured by `attributes`.
    pub fn create_sink_writer(
        &self,
        path: &windows::core::HSTRING,
        attributes: &IMFAttributes,
    ) -> Result<IMFSinkWriter, CaptureError> {
        let mut p = std::ptr::null_mut();
        // SAFETY: `path` is a live NUL-terminated wide string; no byte stream
        // (null); the attributes outlive the call.
        unsafe {
            (self.create_sink_writer_from_url)(
                PCWSTR(path.as_ptr()),
                std::ptr::null_mut(),
                attributes.as_raw(),
                &raw mut p,
            )
        }
        .ok()
        .context("MFCreateSinkWriterFromURL")?;
        // SAFETY: as above.
        unsafe { owned(p, "MFCreateSinkWriterFromURL") }
    }
}

/// Media Foundation started; shut down again when dropped. Start/shutdown
/// are reference-counted by Media Foundation, so sessions may nest.
pub struct MfSession {
    mf: &'static Mf,
}

impl MfSession {
    pub fn mf(&self) -> &'static Mf {
        self.mf
    }
}

impl Drop for MfSession {
    fn drop(&mut self) {
        // SAFETY: balances the successful MFStartup in `Mf::session`.
        let _ = unsafe { (self.mf.shutdown)() };
    }
}
