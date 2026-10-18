//! v1.55.0: charm pictures — the PNG, JPEG, GIF and WebP files the stream
//! overlay shows.
//!
//! `import_overlay_asset` copies a picture into `overlay/assets/`, named by
//! its content (`<16 hex>.<ext>`), so no path ever reaches the scene;
//! `delete_overlay_asset` removes one; and [`serve`] answers the overlay's
//! `http://charm.localhost/<id>` requests on the `charm` scheme.
//!
//! Every header read is bounds-checked: a truncated or hostile file is an
//! error, never a panic.

use std::fmt::Write as _;
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Condvar, Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::http::header::{ALLOW, CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS};
use tauri::http::{HeaderValue, Response, StatusCode};

use crate::overlay_machine::OVERLAY_LABEL;

/// The scheme charm pictures are served on: `http://charm.localhost/<id>`
/// on Windows.
pub const CHARM_SCHEME: &str = "charm";

/// The largest picture file: 50 MB, counted as Replay's UI counts MB.
pub const MAX_BYTES: u64 = 50_000_000;
/// Above this, the import carries the large-picture warning.
pub const WARN_BYTES: u64 = 20_000_000;
/// The longest side a picture may have, in pixels.
pub const MAX_SIDE: u32 = 8192;
/// The most pixels a picture may have (16 megapixels).
pub const MAX_PIXELS: u64 = 16_777_216;
/// The most pixels an animated picture may have (4 megapixels): a
/// full-screen 1440p GIF fits.
pub const MAX_ANIMATED_PIXELS: u64 = 4_194_304;
/// Above this, an animated picture carries the warning (2 megapixels).
pub const WARN_ANIMATED_PIXELS: u64 = 2_097_152;

/// How many pictures the scheme reads and holds at once.
pub const SERVE_SLOTS: usize = 2;

/// What the large-picture warning says.
const LARGE_WARNING: &str =
    "Picture added. It's a large picture, so the overlay may use more memory and CPU.";

/// A picture format the overlay shows. An APNG is a PNG.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl Kind {
    /// The stored file's extension.
    pub fn ext(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
            Self::Gif => "gif",
            Self::Webp => "webp",
        }
    }

    /// What the scheme serves it as.
    pub fn content_type(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }

    /// The kind a stored file's extension names.
    pub fn from_ext(e: &str) -> Option<Self> {
        match e {
            "png" => Some(Self::Png),
            "jpg" => Some(Self::Jpeg),
            "gif" => Some(Self::Gif),
            "webp" => Some(Self::Webp),
            _ => None,
        }
    }
}

/// What [`inspect`] found in a picture's header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inspected {
    pub kind: Kind,
    /// The size as displayed, in the picture's own pixels.
    pub width: u32,
    pub height: u32,
    pub animated: bool,
}

/// Why a picture couldn't be added.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImportError {
    NotAFile,
    TooLarge,
    NotAPicture,
    Damaged,
    UnsupportedJpeg,
    TooManyPixels,
    TooManyAnimatedPixels,
    Read(String),
    Write(String),
}

impl ImportError {
    /// The refusal in the user's words.
    pub fn message(&self) -> String {
        match self {
            Self::NotAFile => "That isn't a picture file.".to_owned(),
            Self::TooLarge => "That picture is over 50 MB. Pick a smaller one.".to_owned(),
            Self::NotAPicture => {
                "Divora can add PNG, JPEG, GIF and WebP pictures. That file isn't one.".to_owned()
            }
            Self::Damaged => "That picture looks damaged, so Divora can't add it.".to_owned(),
            Self::UnsupportedJpeg => "Divora can't show that kind of JPEG. Save it as a regular JPEG or a PNG and try again.".to_owned(),
            Self::TooManyPixels => "That picture is too big (over 16 megapixels or 8192 pixels on a side). Make it smaller and try again.".to_owned(),
            Self::TooManyAnimatedPixels => "That animated picture is too big (over 4 megapixels). Make it smaller and try again.".to_owned(),
            Self::Read(e) => format!("Couldn't read that picture: {e}"),
            Self::Write(e) => format!("Couldn't save the picture: {e}"),
        }
    }
}

/// What `import_overlay_asset` returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportedAsset {
    /// The stored file's name, `<16 hex>.<png|jpg|gif|webp>`.
    pub id: String,
    /// The picture's size as it is displayed, in its own pixels.
    pub width: u32,
    pub height: u32,
    /// The file's size.
    pub bytes: u64,
    pub animated: bool,
    /// A note for a large picture, in the user's words.
    pub warning: Option<String>,
}

// ---- Reading headers ----

/// `N` bytes at `at`, or `None` past the end.
fn bytes_at<const N: usize>(b: &[u8], at: usize) -> Option<[u8; N]> {
    b.get(at..at.checked_add(N)?)?.try_into().ok()
}

fn be16(b: &[u8], at: usize) -> Option<u16> {
    bytes_at(b, at).map(u16::from_be_bytes)
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    bytes_at(b, at).map(u16::from_le_bytes)
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    bytes_at(b, at).map(u32::from_be_bytes)
}

fn le24(b: &[u8], at: usize) -> Option<u32> {
    bytes_at::<3>(b, at).map(|[x, y, z]| u32::from_le_bytes([x, y, z, 0]))
}

/// The picture format its first bytes name. Nothing else is accepted: not
/// SVG, BMP, ICO, TIFF, AVIF or HEIC.
#[doc(hidden)]
pub fn sniff(bytes: &[u8]) -> Option<Kind> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(Kind::Png)
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some(Kind::Jpeg)
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(Kind::Gif)
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(&b"WEBP"[..]) {
        Some(Kind::Webp)
    } else {
        None
    }
}

/// The picture's size as it is displayed: `Damaged` for truncated or bad
/// input, `UnsupportedJpeg` for a JPEG Chromium won't draw. Never a 0 side.
#[doc(hidden)]
pub fn dimensions(kind: Kind, bytes: &[u8]) -> Result<(u32, u32), ImportError> {
    let size = match kind {
        Kind::Png => png_size(bytes),
        Kind::Gif => gif_canvas(bytes),
        Kind::Jpeg => Some(jpeg_size(bytes)?),
        Kind::Webp => webp_size(bytes),
    };
    // One rule for every format: a 0 side would become a 0 or NaN rectangle.
    size.filter(|&(w, h)| w > 0 && h > 0)
        .ok_or(ImportError::Damaged)
}

fn png_size(b: &[u8]) -> Option<(u32, u32)> {
    if b.get(12..16) != Some(&b"IHDR"[..]) {
        return None;
    }
    Some((be32(b, 16)?, be32(b, 20)?))
}

/// The size of a GIF colour table, from its packed flags byte.
fn colour_table_len(packed: u8) -> usize {
    if packed & 0x80 == 0 {
        0
    } else {
        3 << ((packed & 7) + 1)
    }
}

/// Where a GIF's blocks start: after the header, the logical screen and the
/// global colour table.
fn gif_blocks(b: &[u8]) -> Option<usize> {
    Some(13 + colour_table_len(*b.get(10)?))
}

/// Skip the GIF sub-blocks starting at `p`, up to and including the empty
/// one that ends them.
fn skip_sub_blocks(b: &[u8], mut p: usize) -> Option<usize> {
    loop {
        let n = usize::from(*b.get(p)?);
        p = p.checked_add(1 + n)?;
        if n == 0 {
            return Some(p);
        }
    }
}

/// A GIF's canvas: the logical screen grown to hold the first frame. This is
/// Wuffs' default, which Chromium uses; later frames are clipped to it.
#[doc(hidden)]
pub fn gif_canvas(b: &[u8]) -> Option<(u32, u32)> {
    let sw = u32::from(le16(b, 6)?);
    let sh = u32::from(le16(b, 8)?);
    let mut p = gif_blocks(b)?;
    loop {
        match *b.get(p)? {
            // An extension: introducer, label, sub-blocks.
            0x21 => p = skip_sub_blocks(b, p.checked_add(2)?)?,
            // The first image descriptor.
            0x2C => {
                let field = |offset: usize| le16(b, p.checked_add(offset)?).map(u32::from);
                let (left, top) = (field(1)?, field(3)?);
                let (fw, fh) = (field(5)?, field(7)?);
                return Some((sw.max(left + fw), sh.max(top + fh)));
            }
            // The trailer, or anything else: no frame to show.
            _ => return None,
        }
    }
}

/// Whether a GIF has a second frame; `None` when the walk can't finish.
fn gif_animated(b: &[u8]) -> Option<bool> {
    let mut p = gif_blocks(b)?;
    let mut seen_frame = false;
    loop {
        match *b.get(p)? {
            0x21 => p = skip_sub_blocks(b, p.checked_add(2)?)?,
            0x2C if seen_frame => return Some(true),
            0x2C => {
                seen_frame = true;
                let packed = *b.get(p.checked_add(9)?)?;
                // The descriptor, its local colour table and the LZW
                // minimum code size, then the image data's sub-blocks.
                p = p.checked_add(10 + colour_table_len(packed) + 1)?;
                p = skip_sub_blocks(b, p)?;
            }
            0x3B => return Some(false),
            _ => return None,
        }
    }
}

/// Whether a PNG is an APNG: `acTL` before the first `IDAT`. `None` when
/// the walk can't reach either.
fn png_animated(b: &[u8]) -> Option<bool> {
    let mut p = 8_usize;
    loop {
        let len = usize::try_from(be32(b, p)?).ok()?;
        match b.get(p.checked_add(4)?..p.checked_add(8)?)? {
            b"acTL" => return Some(true),
            b"IDAT" => return Some(false),
            // Length, type, data, CRC.
            _ => p = p.checked_add(12)?.checked_add(len)?,
        }
    }
}

fn webp_size(b: &[u8]) -> Option<(u32, u32)> {
    match b.get(12..16)? {
        b"VP8 " => {
            if b.get(23..26) != Some(&[0x9D, 0x01, 0x2A][..]) {
                return None;
            }
            // 14 bits each; the top two are a scale the decoder ignores.
            let w = le16(b, 26)? & 0x3FFF;
            let h = le16(b, 28)? & 0x3FFF;
            Some((u32::from(w), u32::from(h)))
        }
        b"VP8L" => {
            if *b.get(20)? != 0x2F {
                return None;
            }
            // 14 bits of width - 1, then 14 bits of height - 1.
            let [b0, b1, b2, b3] = bytes_at::<4>(b, 21)?.map(u32::from);
            let w = 1 + (b0 | ((b1 & 0x3F) << 8));
            let h = 1 + ((b1 >> 6) | (b2 << 2) | ((b3 & 0x0F) << 10));
            Some((w, h))
        }
        b"VP8X" => Some((1 + le24(b, 24)?, 1 + le24(b, 27)?)),
        _ => None,
    }
}

/// A frame header (SOF). DHT, JPG and DAC share its range but aren't.
fn is_sof(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC)
}

/// A JPEG's displayed size: the first SOF's, swapped for EXIF orientations
/// 5-8. Only baseline and progressive 8-bit JPEGs (SOF0/1/2, precision 8)
/// are accepted: Chromium's libjpeg-turbo has no arithmetic decoder, and
/// Blink uses only the 8-bit API.
fn jpeg_size(b: &[u8]) -> Result<(u32, u32), ImportError> {
    let mut p = 2;
    let mut size = None;
    let mut orientation = None;
    // Walk to the first scan, as libjpeg reads the header, so an EXIF block
    // after the SOF still counts. A walk that stops early keeps what it has.
    loop {
        if b.get(p) != Some(&0xFF) {
            break;
        }
        // Any further FFs are fill.
        while b.get(p) == Some(&0xFF) {
            p += 1;
        }
        let Some(&marker) = b.get(p) else { break };
        p += 1;
        match marker {
            // No length.
            0xD8 | 0xD0..=0xD7 | 0x01 => continue,
            // End of image, start of scan.
            0xD9 | 0xDA => break,
            _ => {}
        }
        let Some(len) = be16(b, p).map(usize::from).filter(|&n| n >= 2) else {
            break;
        };
        let Some(segment) = b.get(p + 2..p + len) else {
            break;
        };
        p += len;
        if size.is_none() && is_sof(marker) {
            // The first SOF decides: precision, height, width.
            let (Some(&precision), Some(h), Some(w)) =
                (segment.first(), be16(segment, 1), be16(segment, 3))
            else {
                return Err(ImportError::Damaged);
            };
            if !matches!(marker, 0xC0..=0xC2) || precision != 8 {
                return Err(ImportError::UnsupportedJpeg);
            }
            size = Some((u32::from(w), u32::from(h)));
        } else if marker == 0xE1 && orientation.is_none() {
            if let Some(tiff) = segment.strip_prefix(b"Exif\0\0") {
                orientation = jpeg_orientation(tiff);
            }
        }
    }
    let (w, h) = size.ok_or(ImportError::Damaged)?;
    Ok(if matches!(orientation, Some(5..=8)) {
        (h, w)
    } else {
        (w, h)
    })
}

/// The EXIF orientation in a TIFF block (what follows `Exif\0\0` in a
/// JPEG's APP1): IFD0's tag 0x0112, a SHORT with a count of 1.
#[doc(hidden)]
pub fn jpeg_orientation(tiff: &[u8]) -> Option<u16> {
    let big = match tiff.get(0..2)? {
        b"MM" => true,
        b"II" => false,
        _ => return None,
    };
    let u16_at = |at: usize| {
        bytes_at(tiff, at).map(|v| {
            if big {
                u16::from_be_bytes(v)
            } else {
                u16::from_le_bytes(v)
            }
        })
    };
    let u32_at = |at: usize| {
        bytes_at(tiff, at).map(|v| {
            if big {
                u32::from_be_bytes(v)
            } else {
                u32::from_le_bytes(v)
            }
        })
    };
    if u16_at(2)? != 0x2A {
        return None;
    }
    let ifd = usize::try_from(u32_at(4)?).ok()?;
    let count = usize::from(u16_at(ifd)?);
    for i in 0..count {
        // Entries of 12 bytes: tag, type, count, value.
        let entry = ifd.checked_add(2 + 12 * i)?;
        if u16_at(entry)? == 0x0112 && u16_at(entry + 2)? == 3 && u32_at(entry + 4)? == 1 {
            return u16_at(entry + 8);
        }
    }
    None
}

/// Whether a picture has more than one frame. A GIF or PNG walk that can't
/// finish counts as animated, so the stricter cap applies.
#[doc(hidden)]
pub fn animated(kind: Kind, bytes: &[u8]) -> bool {
    match kind {
        Kind::Gif => gif_animated(bytes).unwrap_or(true),
        Kind::Png => png_animated(bytes).unwrap_or(true),
        Kind::Webp => {
            bytes.get(12..16) == Some(&b"VP8X"[..]) && bytes.get(20).is_some_and(|f| f & 0x02 != 0)
        }
        Kind::Jpeg => false,
    }
}

/// Sniff, size and cap a picture.
#[doc(hidden)]
pub fn inspect(bytes: &[u8]) -> Result<Inspected, ImportError> {
    let kind = sniff(bytes).ok_or(ImportError::NotAPicture)?;
    let (width, height) = dimensions(kind, bytes)?;
    let animated = animated(kind, bytes);
    let pixels = u64::from(width) * u64::from(height);
    if width > MAX_SIDE || height > MAX_SIDE || pixels > MAX_PIXELS {
        return Err(ImportError::TooManyPixels);
    }
    if animated && pixels > MAX_ANIMATED_PIXELS {
        return Err(ImportError::TooManyAnimatedPixels);
    }
    Ok(Inspected {
        kind,
        width,
        height,
        animated,
    })
}

/// The note for a picture over 20 MB, or an animated one over 2 megapixels.
#[doc(hidden)]
pub fn large_warning(bytes: u64, animated: bool, width: u32, height: u32) -> Option<String> {
    let pixels = u64::from(width) * u64::from(height);
    (bytes > WARN_BYTES || (animated && pixels > WARN_ANIMATED_PIXELS))
        .then(|| LARGE_WARNING.to_owned())
}

/// A picture's id: the first 16 hex digits of its SHA-256, a dot, and the
/// sniffed type's extension.
#[doc(hidden)]
pub fn asset_id(bytes: &[u8], kind: Kind) -> String {
    let digest = Sha256::digest(bytes);
    let mut id = String::with_capacity(21);
    for b in &digest[..8] {
        let _ = write!(id, "{b:02x}");
    }
    id.push('.');
    id.push_str(kind.ext());
    id
}

/// Whether `s` is a charm picture's id: 16 lowercase hex digits, a dot, and
/// `png`, `jpg`, `gif` or `webp`. Nothing else names a file the scheme
/// serves or `delete` removes.
#[doc(hidden)]
pub fn is_asset_id(s: &str) -> bool {
    let Some((hex, ext)) = s.split_once('.') else {
        return false;
    };
    hex.len() == 16
        && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        && matches!(ext, "png" | "jpg" | "gif" | "webp")
}

/// The pictures' folder under `overlay/`. Development builds keep their own
/// scene, so they keep their own pictures: one build can't delete the
/// other's.
#[doc(hidden)]
pub fn assets_dir_name(dev: bool) -> &'static str {
    if dev {
        "assets-dev"
    } else {
        "assets"
    }
}

/// Whether `path` is a file of exactly `buf`'s bytes. Any error is `false`.
#[doc(hidden)]
pub fn same_contents(path: &Path, buf: &[u8]) -> bool {
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    // Only a file of the same length is read at all.
    match file.metadata() {
        Ok(m) if m.is_file() && m.len() == buf.len() as u64 => {}
        _ => return false,
    }
    let mut chunk = vec![0_u8; 64 * 1024];
    let mut rest = buf;
    loop {
        match file.read(&mut chunk) {
            Ok(0) => return rest.is_empty(),
            Ok(n) => match (chunk.get(..n), rest.get(..n)) {
                (Some(read), Some(want)) if read == want => rest = &rest[n..],
                _ => return false,
            },
            Err(e) if e.kind() == ErrorKind::Interrupted => {}
            Err(_) => return false,
        }
    }
}

/// The whole file, or `None` when it is over [`MAX_BYTES`]: by its size, or
/// because it grew while being read.
fn read_capped(path: &Path) -> std::io::Result<Option<Vec<u8>>> {
    let file = File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_BYTES {
        return Ok(None);
    }
    let mut buf = Vec::with_capacity(usize::try_from(len).unwrap_or(0));
    file.take(MAX_BYTES + 1).read_to_end(&mut buf)?;
    Ok((buf.len() as u64 <= MAX_BYTES).then_some(buf))
}

/// Bounds how many pictures the scheme holds in memory at once.
#[doc(hidden)]
pub struct ServeGate {
    free: Mutex<usize>,
    freed: Condvar,
}

/// A place in a [`ServeGate`], given back when dropped.
#[doc(hidden)]
#[must_use]
pub struct ServeSlot<'a> {
    gate: &'a ServeGate,
}

impl ServeGate {
    pub fn new(slots: usize) -> Self {
        Self {
            free: Mutex::new(slots),
            freed: Condvar::new(),
        }
    }

    /// Wait for a place.
    pub fn acquire(&self) -> ServeSlot<'_> {
        let free = self.free.lock().unwrap_or_else(PoisonError::into_inner);
        let mut free = self
            .freed
            .wait_while(free, |n| *n == 0)
            .unwrap_or_else(PoisonError::into_inner);
        *free -= 1;
        ServeSlot { gate: self }
    }
}

impl Drop for ServeSlot<'_> {
    fn drop(&mut self) {
        *self
            .gate
            .free
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
        self.gate.freed.notify_one();
    }
}

/// Nanoseconds since 1970, never the same twice in this process, so two
/// imports at once never share a temp file.
fn unique_nanos() -> u64 {
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_nanos()).unwrap_or(u64::MAX));
    let mut last = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(last.saturating_add(1));
        match LAST.compare_exchange_weak(last, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(seen) => last = seen,
        }
    }
}

fn write_synced(mut file: File, buf: &[u8]) -> std::io::Result<()> {
    file.write_all(buf)?;
    // The data is on disk before the rename, so a power cut can't leave a
    // zeroed picture under its id.
    file.sync_all()
}

/// Copy the picture at `src` into `dir`, named by its content.
pub fn import(dir: &Path, src: &Path) -> Result<ImportedAsset, ImportError> {
    let read_err = |e: std::io::Error| ImportError::Read(e.to_string());
    let write_err = |e: std::io::Error| ImportError::Write(e.to_string());

    let meta = fs::metadata(src).map_err(read_err)?;
    // A folder, a device, or nothing at all.
    if !meta.is_file() || meta.len() == 0 {
        return Err(ImportError::NotAFile);
    }
    if meta.len() > MAX_BYTES {
        return Err(ImportError::TooLarge);
    }
    let buf = read_capped(src)
        .map_err(read_err)?
        .ok_or(ImportError::TooLarge)?;
    let found = inspect(&buf)?;
    let bytes = buf.len() as u64;
    let asset = ImportedAsset {
        id: asset_id(&buf, found.kind),
        width: found.width,
        height: found.height,
        bytes,
        animated: found.animated,
        warning: large_warning(bytes, found.animated, found.width, found.height),
    };
    let dest = dir.join(&asset.id);

    fs::create_dir_all(dir).map_err(write_err)?;
    // An intact copy is never rewritten. A damaged, truncated or different
    // file under that name is replaced.
    if same_contents(&dest, &buf) {
        return Ok(asset);
    }
    // The temp file sits beside the pictures' folder, not in it, and its
    // name can never be an id.
    let beside = dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(dir);
    let temp = beside.join(format!(
        ".import-{}-{}.part",
        std::process::id(),
        unique_nanos()
    ));
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(write_err)?;
    if let Err(e) = write_synced(file, &buf) {
        let _ = fs::remove_file(&temp);
        return Err(write_err(e));
    }
    // Replaces a damaged copy. The scheme opens files with share-delete, so
    // a read in progress doesn't block it.
    if let Err(e) = fs::rename(&temp, &dest) {
        let _ = fs::remove_file(&temp);
        // An import of the same picture got there first.
        if same_contents(&dest, &buf) {
            return Ok(asset);
        }
        return Err(write_err(e));
    }
    Ok(asset)
}

/// Remove the picture `id` from `dir`. A missing file is fine.
pub fn delete(dir: &Path, id: &str) -> Result<(), String> {
    if !is_asset_id(id) {
        return Err("That isn't a charm picture.".to_owned());
    }
    match fs::remove_file(dir.join(id)) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("Couldn't remove the picture: {e}")),
    }
}

/// An answer with no body.
fn empty(status: StatusCode) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = status;
    response
}

/// Answer one `charm` request from the webview labelled `webview`. `dir` is
/// `None` until the app's state exists; `live` says whether the overlay is
/// still open.
pub fn serve(
    dir: Option<&Path>,
    webview: &str,
    method: &str,
    path: &str,
    live: impl Fn() -> bool,
) -> Response<Vec<u8>> {
    let Some(dir) = dir else {
        return empty(StatusCode::SERVICE_UNAVAILABLE);
    };
    // Settings shows no thumbnails, so only the overlay loads pictures.
    if webview != OVERLAY_LABEL {
        return empty(StatusCode::FORBIDDEN);
    }
    if method != "GET" {
        let mut response = empty(StatusCode::METHOD_NOT_ALLOWED);
        response
            .headers_mut()
            .insert(ALLOW, HeaderValue::from_static("GET"));
        return response;
    }
    let Some(id) = path.strip_prefix('/').filter(|id| is_asset_id(id)) else {
        return empty(StatusCode::NOT_FOUND);
    };
    let body = match read_capped(&dir.join(id)) {
        Ok(Some(body)) => body,
        Ok(None) => {
            tracing::warn!(id, "charm asset is over the size cap");
            return empty(StatusCode::NOT_FOUND);
        }
        Err(e) if e.kind() == ErrorKind::NotFound => return empty(StatusCode::NOT_FOUND),
        Err(e) => {
            tracing::warn!(id, error = %e, "couldn't read a charm asset");
            return empty(StatusCode::INTERNAL_SERVER_ERROR);
        }
    };
    let ext = id.rsplit_once('.').map_or("", |(_, ext)| ext);
    let Some(kind) = sniff(&body).filter(|&k| Kind::from_ext(ext) == Some(k)) else {
        tracing::warn!(id, "charm asset doesn't match its type");
        return empty(StatusCode::NOT_FOUND);
    };
    // Checked last, just before answering: a closing overlay is handed
    // nothing to hold.
    if !live() {
        return empty(StatusCode::SERVICE_UNAVAILABLE);
    }
    let mut response = Response::new(body);
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(kind.content_type()));
    // An id names its content, so a picture never changes under its URL.
    headers.insert(
        CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    response
}

#[cfg(test)]
mod tests {
    use super::{
        animated, asset_id, assets_dir_name, delete, dimensions, gif_canvas, import, inspect,
        is_asset_id, jpeg_orientation, large_warning, same_contents, serve, sniff, ImportError,
        ImportedAsset, Kind, ServeGate, CHARM_SCHEME, MAX_BYTES, SERVE_SLOTS,
    };
    use serde_json::Value;
    use std::fs::{self, File};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, SystemTime};
    use tauri::http::header::{
        HeaderName, ALLOW, CACHE_CONTROL, CONTENT_TYPE, X_CONTENT_TYPE_OPTIONS,
    };
    use tauri::http::{Response, StatusCode};

    const KINDS: [Kind; 4] = [Kind::Png, Kind::Jpeg, Kind::Gif, Kind::Webp];

    /// The wire contract both test suites read.
    fn fixture() -> Value {
        serde_json::from_str(include_str!("../../src/overlay/wire.fixture.json")).unwrap()
    }

    // ---- Pictures, built in code ----

    fn png_chunk(out: &mut Vec<u8>, ty: [u8; 4], data: &[u8]) {
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_be_bytes());
        out.extend_from_slice(&ty);
        out.extend_from_slice(data);
        out.extend_from_slice(&[0; 4]); // the CRC isn't checked
    }

    /// A PNG of `w`×`h` with these chunks after IHDR.
    fn png_file(w: u32, h: u32, chunks: &[&[u8; 4]]) -> Vec<u8> {
        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        png_chunk(&mut out, *b"IHDR", &ihdr);
        for &ty in chunks {
            let data: &[u8] = match ty {
                b"acTL" => &[0, 0, 0, 2, 0, 0, 0, 0],
                b"IEND" => &[],
                _ => &[b'I', b'D', b'A', b'T', 0x2C],
            };
            png_chunk(&mut out, *ty, data);
        }
        out
    }

    fn png(w: u32, h: u32) -> Vec<u8> {
        png_file(w, h, &[b"IDAT", b"IEND"])
    }

    /// A GIF extension block: introducer, label, sub-blocks, terminator.
    fn gif_ext(label: u8, blocks: &[&[u8]]) -> Vec<u8> {
        let mut out = vec![0x21, label];
        for b in blocks {
            out.push(u8::try_from(b.len()).unwrap());
            out.extend_from_slice(b);
        }
        out.push(0);
        out
    }

    /// One frame: the descriptor, a local colour table of `lct_bits`, the
    /// LZW minimum code size and data that holds block-like bytes.
    fn gif_frame(out: &mut Vec<u8>, rect: [u16; 4], lct_bits: Option<u8>) {
        out.push(0x2C);
        for field in rect {
            out.extend_from_slice(&field.to_le_bytes());
        }
        out.push(lct_bits.map_or(0, |bits| 0x80 | bits));
        if let Some(bits) = lct_bits {
            out.resize(out.len() + (3 << (bits + 1)), 0x2C);
        }
        out.extend_from_slice(&[2, 3, 0x2C, 0x21, 0x3B, 0]);
    }

    /// A GIF: the logical `screen`, a global colour table of `gct_bits`
    /// (filled with 0x2C, so a walk that doesn't skip it misreads), the
    /// extension blocks `exts`, one frame per `(l, t, w, h)`, the trailer.
    fn gif(
        screen: (u16, u16),
        gct_bits: Option<u8>,
        exts: &[Vec<u8>],
        frames: &[[u16; 4]],
    ) -> Vec<u8> {
        let mut out = b"GIF89a".to_vec();
        out.extend_from_slice(&screen.0.to_le_bytes());
        out.extend_from_slice(&screen.1.to_le_bytes());
        out.push(gct_bits.map_or(0, |bits| 0x80 | bits));
        out.extend_from_slice(&[0, 0]);
        if let Some(bits) = gct_bits {
            out.resize(out.len() + (3 << (bits + 1)), 0x2C);
        }
        for e in exts {
            out.extend_from_slice(e);
        }
        for &f in frames {
            gif_frame(&mut out, f, None);
        }
        out.push(0x3B);
        out
    }

    /// A JPEG segment: marker, length, data.
    fn seg(marker: u8, data: &[u8]) -> Vec<u8> {
        let mut out = vec![0xFF, marker];
        out.extend_from_slice(&u16::try_from(data.len() + 2).unwrap().to_be_bytes());
        out.extend_from_slice(data);
        out
    }

    fn sof_data(precision: u8, w: u16, h: u16) -> Vec<u8> {
        let mut data = vec![precision];
        data.extend_from_slice(&h.to_be_bytes());
        data.extend_from_slice(&w.to_be_bytes());
        data.extend_from_slice(&[1, 1, 0x11, 0]);
        data
    }

    /// A JPEG: SOI, an EXIF block holding `exif` (a TIFF block), `segments`,
    /// a frame header `sof` of `precision` and `w`×`h`, a scan, the end.
    fn jpeg(
        sof: u8,
        precision: u8,
        w: u16,
        h: u16,
        segments: &[Vec<u8>],
        exif: Option<&[u8]>,
    ) -> Vec<u8> {
        let mut out = vec![0xFF, 0xD8];
        if let Some(tiff) = exif {
            out.extend(seg(0xE1, &[&b"Exif\0\0"[..], tiff].concat()));
        }
        for s in segments {
            out.extend_from_slice(s);
        }
        out.extend(seg(sof, &sof_data(precision, w, h)));
        out.extend(seg(0xDA, &[1, 1, 0, 0, 0x3F, 0]));
        out.extend_from_slice(&[0x12, 0xFF, 0x00, 0x34, 0xFF, 0xD9]);
        out
    }

    /// A TIFF block in either byte order: a Make tag, then the orientation.
    fn tiff(big: bool, orientation: u16) -> Vec<u8> {
        let u16b = |v: u16| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let u32b = |v: u32| {
            if big {
                v.to_be_bytes()
            } else {
                v.to_le_bytes()
            }
        };
        let mut out = if big { b"MM".to_vec() } else { b"II".to_vec() };
        out.extend(u16b(0x2A));
        out.extend(u32b(8));
        out.extend(u16b(2));
        out.extend(u16b(0x010F));
        out.extend(u16b(2));
        out.extend(u32b(4));
        out.extend_from_slice(b"abc\0");
        out.extend(u16b(0x0112));
        out.extend(u16b(3));
        out.extend(u32b(1));
        out.extend(u16b(orientation));
        out.extend([0, 0]);
        out.extend(u32b(0));
        out
    }

    fn webp_chunk(fourcc: [u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = fourcc.to_vec();
        out.extend_from_slice(&u32::try_from(data.len()).unwrap().to_le_bytes());
        out.extend_from_slice(data);
        if data.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn riff(chunks: &[u8]) -> Vec<u8> {
        let mut out = b"RIFF".to_vec();
        out.extend_from_slice(&u32::try_from(chunks.len() + 4).unwrap().to_le_bytes());
        out.extend_from_slice(b"WEBP");
        out.extend_from_slice(chunks);
        out
    }

    /// A lossy WebP. `w` and `h` are the raw 16-bit fields (scale included).
    fn webp_vp8(w: u16, h: u16) -> Vec<u8> {
        let mut data = vec![0x50, 0x02, 0x00, 0x9D, 0x01, 0x2A];
        data.extend_from_slice(&w.to_le_bytes());
        data.extend_from_slice(&h.to_le_bytes());
        riff(&webp_chunk(*b"VP8 ", &data))
    }

    fn webp_vp8l(w: u32, h: u32) -> Vec<u8> {
        let bits = (w - 1) | ((h - 1) << 14);
        let mut data = vec![0x2F];
        data.extend_from_slice(&bits.to_le_bytes());
        riff(&webp_chunk(*b"VP8L", &data))
    }

    /// An extended WebP with these flags (0x02 is animation).
    fn webp_vp8x(w: u32, h: u32, flags: u8) -> Vec<u8> {
        let mut data = vec![flags, 0, 0, 0];
        data.extend_from_slice(&(w - 1).to_le_bytes()[..3]);
        data.extend_from_slice(&(h - 1).to_le_bytes()[..3]);
        let mut chunks = webp_chunk(*b"VP8X", &data);
        chunks.extend(webp_chunk(*b"ANIM", &[0; 6]));
        riff(&chunks)
    }

    /// One of each, and the awkward cases.
    fn fixtures() -> Vec<Vec<u8>> {
        let comment = gif_ext(0xFE, &[&[b'x'; 255], &[0x2C; 40]]);
        vec![
            png(498, 280),
            png_file(64, 32, &[b"acTL", b"IDAT", b"IEND"]),
            gif(
                (10, 10),
                Some(2),
                &[comment],
                &[[30, 20, 50, 40], [0, 0, 5, 5]],
            ),
            jpeg(
                0xC2,
                8,
                640,
                480,
                &[seg(0xC4, &[0; 20])],
                Some(&tiff(true, 6)),
            ),
            jpeg(0xC0, 8, 1, 1, &[], Some(&tiff(false, 8))),
            webp_vp8(640, 480),
            webp_vp8l(12_345, 6_789),
            webp_vp8x(300, 200, 0x02),
        ]
    }

    // ---- Sniffing and sizes ----

    #[test]
    fn sniff_table() {
        let cases: &[(&[u8], Option<Kind>)] = &[
            (b"\x89PNG\r\n\x1a\n", Some(Kind::Png)),
            (b"\xFF\xD8\xFF\xE0", Some(Kind::Jpeg)),
            (b"GIF87a", Some(Kind::Gif)),
            (b"GIF89a", Some(Kind::Gif)),
            (b"RIFF\0\0\0\0WEBPVP8 ", Some(Kind::Webp)),
            // Near misses.
            (b"\x89PNG\r\n\x1a", None),
            (b"\x89PNG\r\n\x1a\x0b", None),
            (b"\xFF\xD8", None),
            (b"\xFF\xD8\xFE", None),
            (b"GIF88a", None),
            (b"GIF89", None),
            (b"gif89a", None),
            (b"RIFF\0\0\0\0WAVEfmt ", None),
            (b"RIFF\0\0\0\0WEB", None),
            (b"WEBP", None),
            // Formats that aren't accepted.
            (b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>", None),
            (b"<?xml version=\"1.0\"?><svg/>", None),
            (b"<!DOCTYPE html><html>", None),
            (b"BM\x36\0\0\0", None),
            (b"\0\0\x01\0\x01\0", None),
            (b"II*\0\x08\0\0\0", None),
            (b"MM\0*\0\0\0\x08", None),
            (b"\0\0\0\x1cftypavif", None),
            (b"\0\0\0\x18ftypheic", None),
            (b"", None),
        ];
        for &(bytes, want) in cases {
            assert_eq!(sniff(bytes), want, "{bytes:?}");
        }
        // Every truncated prefix of each magic is refused.
        for magic in [
            &b"\x89PNG\r\n\x1a\n"[..],
            b"\xFF\xD8\xFF",
            b"GIF89a",
            b"RIFF\0\0\0\0WEBP",
        ] {
            for n in 0..magic.len() {
                assert_eq!(sniff(&magic[..n]), None, "{:?}", &magic[..n]);
            }
        }
    }

    #[test]
    fn kind_extensions_and_content_types() {
        let rows = [
            (Kind::Png, "png", "image/png"),
            (Kind::Jpeg, "jpg", "image/jpeg"),
            (Kind::Gif, "gif", "image/gif"),
            (Kind::Webp, "webp", "image/webp"),
        ];
        for (kind, ext, content_type) in rows {
            assert_eq!(kind.ext(), ext);
            assert_eq!(kind.content_type(), content_type);
            assert_eq!(Kind::from_ext(ext), Some(kind));
        }
        for ext in ["jpeg", "PNG", "svg", "apng", ""] {
            assert_eq!(Kind::from_ext(ext), None, "{ext:?}");
        }
    }

    #[test]
    fn png_gif_jpeg_webp_dimensions() {
        let ok = |kind: Kind, bytes: &[u8], want: (u32, u32)| {
            assert_eq!(dimensions(kind, bytes), Ok(want), "{kind:?} {want:?}");
        };
        let damaged = |kind: Kind, bytes: &[u8], why: &str| {
            assert_eq!(dimensions(kind, bytes), Err(ImportError::Damaged), "{why}");
        };

        // A 1×1 of each format.
        ok(Kind::Png, &png(1, 1), (1, 1));
        ok(Kind::Gif, &gif((1, 1), None, &[], &[[0, 0, 1, 1]]), (1, 1));
        ok(Kind::Jpeg, &jpeg(0xC0, 8, 1, 1, &[], None), (1, 1));
        ok(Kind::Webp, &webp_vp8(1, 1), (1, 1));
        ok(Kind::Webp, &webp_vp8l(1, 1), (1, 1));
        ok(Kind::Webp, &webp_vp8x(1, 1, 0), (1, 1));

        // Larger ones, using every bit of each field.
        ok(Kind::Png, &png(498, 280), (498, 280));
        ok(
            Kind::Png,
            &png(0x0102_0304, 0x0506_0708),
            (0x0102_0304, 0x0506_0708),
        );
        // VP8's top two bits are a scale, not size.
        ok(
            Kind::Webp,
            &webp_vp8(0x0280 | (1 << 14), 0x01E0 | (3 << 14)),
            (640, 480),
        );
        ok(Kind::Webp, &webp_vp8l(12_345, 6_789), (12_345, 6_789));
        ok(Kind::Webp, &webp_vp8l(16_384, 16_384), (16_384, 16_384));
        ok(Kind::Webp, &webp_vp8l(0x2AAB, 0x1555), (0x2AAB, 0x1555));
        ok(Kind::Webp, &webp_vp8x(16_777_216, 3, 0), (16_777_216, 3));

        // A 0 side is damaged, in every format.
        damaged(Kind::Png, &png(0, 7), "png 0 wide");
        damaged(Kind::Png, &png(7, 0), "png 0 high");
        damaged(Kind::Png, &png(0, 0), "png 0x0");
        damaged(
            Kind::Gif,
            &gif((0, 0), None, &[], &[[0, 0, 0, 0]]),
            "gif 0x0",
        );
        damaged(Kind::Jpeg, &jpeg(0xC0, 8, 0, 7, &[], None), "jpeg 0 wide");
        damaged(
            Kind::Jpeg,
            &jpeg(0xC0, 8, 7, 0, &[], None),
            "jpeg 0 high (DNL)",
        );
        damaged(Kind::Webp, &webp_vp8(0, 7), "vp8 0 wide");
        damaged(Kind::Webp, &webp_vp8(7, 0), "vp8 0 high");
        damaged(Kind::Webp, &webp_vp8(1 << 14, 7), "vp8 only a scale");

        // GIF: the logical screen grown to hold the first frame.
        let g = |screen, frame| gif(screen, None, &[], &[frame]);
        ok(Kind::Gif, &g((100, 100), [0, 0, 8000, 2000]), (8000, 2000));
        ok(Kind::Gif, &g((10, 10), [30, 20, 50, 40]), (80, 60));
        ok(Kind::Gif, &g((0, 0), [0, 0, 498, 280]), (498, 280));
        ok(Kind::Gif, &g((320, 240), [0, 0, 0, 0]), (320, 240));
        ok(Kind::Gif, &g((320, 240), [10, 10, 20, 20]), (320, 240));
        // Later frames don't grow it.
        ok(
            Kind::Gif,
            &gif((10, 10), None, &[], &[[0, 0, 10, 10], [0, 0, 900, 900]]),
            (10, 10),
        );
        // A 768-byte global colour table (size bits 7) is skipped.
        ok(
            Kind::Gif,
            &gif((5, 7), Some(7), &[], &[[0, 0, 1, 1]]),
            (5, 7),
        );
        // A large comment, an XMP application extension and a graphic
        // control extension before the frame.
        let comment = gif_ext(0xFE, &[&[b'x'; 255], &[0x2C; 255], &[b'z'; 7]]);
        let xmp = gif_ext(0xFF, &[b"XMP DataXMP", &[0x2C; 200], &[0x3B; 3]]);
        let gce = gif_ext(0xF9, &[&[4, 0, 0, 0]]);
        let busy = gif((20, 10), Some(0), &[comment, xmp, gce], &[[0, 0, 30, 5]]);
        ok(Kind::Gif, &busy, (30, 10));
        assert_eq!(gif_canvas(&busy), Some((30, 10)));
        // Truncated inside an extension sub-block.
        damaged(Kind::Gif, &busy[..13 + 6 + 100], "gif cut in a comment");
        // The trailer before any frame.
        damaged(
            Kind::Gif,
            &gif((10, 10), None, &[], &[]),
            "gif with no frame",
        );
        // A block that is neither an extension nor a frame.
        let mut stray = gif((10, 10), None, &[], &[[0, 0, 1, 1]]);
        stray[13] = 0x00;
        damaged(Kind::Gif, &stray, "gif stray block");

        // PNG: IHDR must come first.
        let mut no_ihdr = png(5, 5);
        no_ihdr[12..16].copy_from_slice(b"IHDX");
        damaged(Kind::Png, &no_ihdr, "png without IHDR");

        // WebP: an unknown chunk, a bad VP8 start code, a bad VP8L signature.
        let mut vp9 = webp_vp8(5, 5);
        vp9[12..16].copy_from_slice(b"VP9 ");
        damaged(Kind::Webp, &vp9, "webp unknown chunk");
        let mut start = webp_vp8(5, 5);
        start[25] = 0x2B;
        damaged(Kind::Webp, &start, "vp8 start code");
        let mut sig = webp_vp8l(5, 5);
        sig[20] = 0x2E;
        damaged(Kind::Webp, &sig, "vp8l signature");
    }

    #[test]
    fn jpeg_sof_table() {
        let size = |sof: u8, precision: u8| {
            dimensions(Kind::Jpeg, &jpeg(sof, precision, 640, 480, &[], None))
        };
        // Baseline, extended and progressive, 8-bit.
        for sof in [0xC0, 0xC1, 0xC2] {
            assert_eq!(size(sof, 8), Ok((640, 480)), "{sof:#X}");
            assert_eq!(
                size(sof, 12),
                Err(ImportError::UnsupportedJpeg),
                "{sof:#X} 12-bit"
            );
            assert_eq!(
                size(sof, 16),
                Err(ImportError::UnsupportedJpeg),
                "{sof:#X} 16-bit"
            );
        }
        // Lossless, hierarchical and arithmetic.
        for sof in [0xC3, 0xC5, 0xC6, 0xC7, 0xC9, 0xCA, 0xCB, 0xCD, 0xCE, 0xCF] {
            assert_eq!(size(sof, 8), Err(ImportError::UnsupportedJpeg), "{sof:#X}");
        }
        // DHT, JPG and DAC sit in the SOF range but are ordinary segments.
        let tables = [seg(0xC4, &[0; 20]), seg(0xC8, &[0; 3]), seg(0xCC, &[0; 4])];
        assert_eq!(
            dimensions(Kind::Jpeg, &jpeg(0xC0, 8, 640, 480, &tables, None)),
            Ok((640, 480))
        );
        // The first SOF decides.
        let first = [seg(0xC0, &sof_data(8, 10, 20))];
        assert_eq!(
            dimensions(Kind::Jpeg, &jpeg(0xC3, 8, 640, 480, &first, None)),
            Ok((10, 20))
        );
        let first = [seg(0xC3, &sof_data(8, 10, 20))];
        assert_eq!(
            dimensions(Kind::Jpeg, &jpeg(0xC0, 8, 640, 480, &first, None)),
            Err(ImportError::UnsupportedJpeg)
        );
        // Fill bytes and markers without a length before the SOF.
        let mut filled = vec![0xFF, 0xFF, 0xFF];
        filled.extend(seg(0xE0, b"JFIF\0\x01\x02"));
        filled.extend_from_slice(&[0xFF, 0x01, 0xFF, 0xD0]);
        assert_eq!(
            dimensions(Kind::Jpeg, &jpeg(0xC0, 8, 640, 480, &[filled], None)),
            Ok((640, 480))
        );
        // A SOF too short to hold its size.
        let mut short = vec![0xFF, 0xD8];
        short.extend(seg(0xC0, &[8, 0, 1]));
        short.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(dimensions(Kind::Jpeg, &short), Err(ImportError::Damaged));
        // A segment length below 2.
        let mut bad_len = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x01];
        bad_len.extend(seg(0xC0, &sof_data(8, 5, 5)));
        assert_eq!(dimensions(Kind::Jpeg, &bad_len), Err(ImportError::Damaged));
    }

    #[test]
    fn jpeg_exif_orientation_both_byte_orders_1_to_8() {
        for big in [false, true] {
            for o in 1..=8 {
                let block = tiff(big, o);
                assert_eq!(jpeg_orientation(&block), Some(o), "big {big} o {o}");
                let want = if o >= 5 { (480, 640) } else { (640, 480) };
                let picture = jpeg(0xC0, 8, 640, 480, &[], Some(&block));
                assert_eq!(
                    dimensions(Kind::Jpeg, &picture),
                    Ok(want),
                    "big {big} o {o}"
                );
            }
            // Out-of-range values don't swap.
            for o in [0, 9, 0xFFFF] {
                let picture = jpeg(0xC0, 8, 640, 480, &[], Some(&tiff(big, o)));
                assert_eq!(dimensions(Kind::Jpeg, &picture), Ok((640, 480)));
            }
        }
        // Every truncation of the TIFF block before the value reads nothing.
        let block = tiff(false, 6);
        for n in 0..32 {
            assert_eq!(jpeg_orientation(&block[..n]), None, "cut at {n}");
        }
        assert_eq!(jpeg_orientation(&block[..32]), Some(6));
        // The wrong type or count is not an orientation.
        let mut long = tiff(false, 6);
        long[24] = 4;
        assert_eq!(jpeg_orientation(&long), None);
        let mut two = tiff(true, 6);
        two[29] = 2;
        assert_eq!(jpeg_orientation(&two), None);
        // A bad byte order or magic.
        let mut order = tiff(false, 6);
        order[0] = b'X';
        assert_eq!(jpeg_orientation(&order), None);
        let mut magic = tiff(false, 6);
        magic[2] = 0x2B;
        assert_eq!(jpeg_orientation(&magic), None);
        // An IFD offset past the end.
        let mut far = tiff(false, 6);
        far[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(jpeg_orientation(&far), None);

        // An XMP APP1 and a broken EXIF block are passed over; the next
        // EXIF block counts.
        let xmp = seg(0xE1, b"http://ns.adobe.com/xap/1.0/\0<x/>");
        let broken = seg(0xE1, b"Exif\0\0XX");
        let six = seg(0xE1, &[&b"Exif\0\0"[..], &tiff(true, 6)].concat());
        let picture = jpeg(0xC0, 8, 640, 480, &[xmp, broken, six], None);
        assert_eq!(dimensions(Kind::Jpeg, &picture), Ok((480, 640)));
        // An EXIF block after the SOF still counts: libjpeg reads to the scan.
        let mut late = jpeg(0xC0, 8, 640, 480, &[], None);
        let sos = late.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
        let six = seg(0xE1, &[&b"Exif\0\0"[..], &tiff(false, 6)].concat());
        late.splice(sos..sos, six);
        assert_eq!(dimensions(Kind::Jpeg, &late), Ok((480, 640)));
        // The first EXIF orientation wins.
        let upright = seg(0xE1, &[&b"Exif\0\0"[..], &tiff(false, 1)].concat());
        let picture = jpeg(0xC0, 8, 640, 480, &[upright], Some(&tiff(false, 6)));
        assert_eq!(dimensions(Kind::Jpeg, &picture), Ok((480, 640)));
    }

    #[test]
    fn jpeg_sos_before_sof_is_damaged() {
        let mut sos_first = vec![0xFF, 0xD8];
        sos_first.extend(seg(0xDB, &[0; 65]));
        sos_first.extend(seg(0xDA, &[1, 1, 0, 0, 0x3F, 0]));
        sos_first.extend(seg(0xC0, &sof_data(8, 640, 480)));
        sos_first.extend_from_slice(&[0xFF, 0xD9]);
        assert_eq!(
            dimensions(Kind::Jpeg, &sos_first),
            Err(ImportError::Damaged)
        );

        let mut eoi_first = vec![0xFF, 0xD8, 0xFF, 0xD9];
        eoi_first.extend(seg(0xC0, &sof_data(8, 640, 480)));
        assert_eq!(
            dimensions(Kind::Jpeg, &eoi_first),
            Err(ImportError::Damaged)
        );

        assert_eq!(
            dimensions(Kind::Jpeg, b"\xFF\xD8\xFF"),
            Err(ImportError::Damaged)
        );
        // Junk where a marker belongs.
        let mut junk = vec![0xFF, 0xD8, 0x00];
        junk.extend(seg(0xC0, &sof_data(8, 640, 480)));
        assert_eq!(dimensions(Kind::Jpeg, &junk), Err(ImportError::Damaged));
    }

    #[test]
    #[allow(clippy::too_many_lines)] // one table, a row per case
    fn animated_table() {
        let one = [[0, 0, 4, 4]];
        let two = [[0, 0, 4, 4], [1, 1, 2, 2]];
        let gce = gif_ext(0xF9, &[&[4, 0, 10, 0]]);
        let rows: Vec<(&str, Kind, Vec<u8>, bool)> = vec![
            (
                "gif, one frame",
                Kind::Gif,
                gif((4, 4), Some(1), &[], &one),
                false,
            ),
            (
                "gif, two frames",
                Kind::Gif,
                gif((4, 4), Some(1), &[], &two),
                true,
            ),
            (
                "gif, two frames after extensions",
                Kind::Gif,
                gif(
                    (4, 4),
                    None,
                    &[gif_ext(0xFF, &[b"NETSCAPE2.0", &[1, 0, 0]]), gce.clone()],
                    &two,
                ),
                true,
            ),
            (
                "gif, one frame then an extension",
                Kind::Gif,
                {
                    let mut g = gif((4, 4), None, &[], &one);
                    g.pop();
                    g.extend(gce);
                    g.push(0x3B);
                    g
                },
                false,
            ),
            (
                "gif, a local colour table, then a second frame",
                Kind::Gif,
                {
                    let mut g = gif((4, 4), None, &[], &[]);
                    g.pop();
                    gif_frame(&mut g, [0, 0, 4, 4], Some(3));
                    gif_frame(&mut g, [0, 0, 4, 4], None);
                    g.push(0x3B);
                    g
                },
                true,
            ),
            (
                "gif, a local colour table, one frame",
                Kind::Gif,
                {
                    let mut g = gif((4, 4), None, &[], &[]);
                    g.pop();
                    gif_frame(&mut g, [0, 0, 4, 4], Some(3));
                    g.push(0x3B);
                    g
                },
                false,
            ),
            (
                "gif, cut before the trailer",
                Kind::Gif,
                {
                    let mut g = gif((4, 4), None, &[], &one);
                    g.pop();
                    g
                },
                true,
            ),
            (
                "gif, a stray block after the frame",
                Kind::Gif,
                {
                    let mut g = gif((4, 4), None, &[], &one);
                    g.pop();
                    g.push(0x00);
                    g
                },
                true,
            ),
            ("png", Kind::Png, png(4, 4), false),
            (
                "apng, acTL before IDAT",
                Kind::Png,
                png_file(4, 4, &[b"acTL", b"IDAT", b"IEND"]),
                true,
            ),
            (
                "png, acTL after IDAT",
                Kind::Png,
                png_file(4, 4, &[b"IDAT", b"acTL", b"IEND"]),
                false,
            ),
            (
                "png, other chunks before IDAT",
                Kind::Png,
                png_file(4, 4, &[b"gAMA", b"tEXt", b"IDAT", b"IEND"]),
                false,
            ),
            ("png, no IDAT", Kind::Png, png_file(4, 4, &[b"IEND"]), true),
            (
                "png, cut after IHDR",
                Kind::Png,
                png(4, 4)[..33].to_vec(),
                true,
            ),
            (
                "webp vp8x, animated",
                Kind::Webp,
                webp_vp8x(4, 4, 0x02),
                true,
            ),
            (
                "webp vp8x, every other flag",
                Kind::Webp,
                webp_vp8x(4, 4, 0xFD),
                false,
            ),
            ("webp vp8", Kind::Webp, webp_vp8(4, 4), false),
            // VP8L's signature byte has the animation bit set.
            ("webp vp8l", Kind::Webp, webp_vp8l(4, 4), false),
            ("jpeg", Kind::Jpeg, jpeg(0xC0, 8, 4, 4, &[], None), false),
        ];
        for (why, kind, bytes, want) in rows {
            assert_eq!(animated(kind, &bytes), want, "{why}");
        }
    }

    /// Run every parser over `bytes`; only a panic fails.
    fn parse_everything(bytes: &[u8]) {
        let _ = sniff(bytes);
        let _ = inspect(bytes);
        for kind in KINDS {
            let _ = dimensions(kind, bytes);
            let _ = animated(kind, bytes);
        }
        let _ = gif_canvas(bytes);
        let _ = jpeg_orientation(bytes);
    }

    #[test]
    fn every_prefix_never_panics() {
        let mut all = fixtures();
        all.push(tiff(true, 6));
        all.push(tiff(false, 6));
        for f in &all {
            for n in 0..=f.len() {
                parse_everything(&f[..n]);
            }
        }
    }

    /// Corrupted bytes: a seeded walk of random overwrites, so a length or
    /// offset taken from the file can't overflow or index out of bounds.
    #[test]
    fn corrupted_pictures_never_panic() {
        let mut seed = 0x2545_F491_4F6C_DD1D_u64;
        let mut next = move || {
            seed = seed
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            usize::try_from(seed >> 33).unwrap()
        };
        let mut all = fixtures();
        all.push(tiff(true, 6));
        for f in &all {
            for _ in 0..2000 {
                let mut bytes = f.clone();
                for _ in 0..=next() % 4 {
                    let at = next() % bytes.len();
                    // Mostly 0xFF and 0x00: the values that make the
                    // biggest lengths and offsets.
                    bytes[at] = match next() % 3 {
                        0 => 0xFF,
                        1 => 0x00,
                        _ => u8::try_from(next() % 256).unwrap(),
                    };
                }
                parse_everything(&bytes);
            }
        }
    }

    #[test]
    fn caps_side_and_area() {
        let check = |bytes: &[u8], want: Result<(u32, u32, bool), ImportError>, why: &str| {
            let got = inspect(bytes).map(|i| (i.width, i.height, i.animated));
            assert_eq!(got, want, "{why}");
        };
        check(&png(8192, 2048), Ok((8192, 2048, false)), "16 MP exactly");
        check(&png(4096, 4096), Ok((4096, 4096, false)), "16 MP square");
        check(
            &png(8193, 1),
            Err(ImportError::TooManyPixels),
            "a side over 8192",
        );
        check(&png(1, 8193), Err(ImportError::TooManyPixels), "tall");
        check(
            &png(4096, 4097),
            Err(ImportError::TooManyPixels),
            "over 16 MP",
        );
        check(
            &png(u32::MAX, u32::MAX),
            Err(ImportError::TooManyPixels),
            "huge",
        );
        // A GIF's frame can't hide behind a small logical screen.
        check(
            &gif((100, 100), None, &[], &[[0, 0, 8000, 8000]]),
            Err(ImportError::TooManyPixels),
            "gif frame larger than its screen",
        );
        // Animated pictures stop at 4 megapixels.
        let anim = |w, h| gif((w, h), None, &[], &[[0, 0, w, h], [0, 0, 1, 1]]);
        check(
            &anim(2048, 2048),
            Ok((2048, 2048, true)),
            "animated 4 MP exactly",
        );
        check(
            &anim(2049, 2048),
            Err(ImportError::TooManyAnimatedPixels),
            "animated gif over 4 MP",
        );
        check(
            &gif((2049, 2048), None, &[], &[[0, 0, 2049, 2048]]),
            Ok((2049, 2048, false)),
            "still gif over 4 MP",
        );
        check(
            &png_file(2049, 2048, &[b"acTL", b"IDAT"]),
            Err(ImportError::TooManyAnimatedPixels),
            "apng over 4 MP",
        );
        check(
            &webp_vp8x(2049, 2048, 0x02),
            Err(ImportError::TooManyAnimatedPixels),
            "animated webp over 4 MP",
        );
        // The caps apply to the size as displayed.
        check(
            &jpeg(0xC0, 8, 8192, 100, &[], Some(&tiff(false, 6))),
            Ok((100, 8192, false)),
            "rotated jpeg",
        );
        // The other refusals pass through.
        check(b"<svg/>", Err(ImportError::NotAPicture), "svg");
        check(&png(0, 1), Err(ImportError::Damaged), "zero side");
        check(
            &jpeg(0xC3, 8, 1, 1, &[], None),
            Err(ImportError::UnsupportedJpeg),
            "lossless jpeg",
        );
    }

    #[test]
    fn large_warning_table() {
        let warning = Some(
            "Picture added. It's a large picture, so the overlay may use more memory and CPU."
                .to_owned(),
        );
        let rows = [
            (20_000_000, false, 10, 10, None),
            (20_000_001, false, 10, 10, warning.clone()),
            (20_000_001, true, 10, 10, warning.clone()),
            (1_000, false, 4096, 4096, None),
            (1_000, true, 2048, 1024, None),
            (1_000, true, 2048, 1025, warning.clone()),
            (1_000, true, 1448, 1448, None),
            (0, true, 2049, 2048, warning),
        ];
        for (bytes, animated, w, h, want) in rows {
            assert_eq!(
                large_warning(bytes, animated, w, h),
                want,
                "{bytes} {animated} {w}x{h}"
            );
        }
    }

    // ---- Ids ----

    #[test]
    fn asset_id_known_vector() {
        // sha256("abc") = ba7816bf8f01cfea414140de5dae2223…
        assert_eq!(asset_id(b"abc", Kind::Png), "ba7816bf8f01cfea.png");
        assert_eq!(asset_id(b"abc", Kind::Jpeg), "ba7816bf8f01cfea.jpg");
        assert_eq!(asset_id(b"abc", Kind::Gif), "ba7816bf8f01cfea.gif");
        assert_eq!(asset_id(b"abc", Kind::Webp), "ba7816bf8f01cfea.webp");
        // sha256("") = e3b0c44298fc1c14…
        assert_eq!(asset_id(b"", Kind::Gif), "e3b0c44298fc1c14.gif");
        for f in fixtures() {
            let id = asset_id(&f, sniff(&f).unwrap());
            assert!(is_asset_id(&id), "{id}");
        }
    }

    /// The same cases `scene.test.ts` checks `ASSET_ID_RE` with.
    #[test]
    fn asset_id_table() {
        let f = fixture();
        let cases = f["assetIds"].as_array().unwrap();
        assert!(cases.len() >= 12);
        for case in cases {
            let id = case["id"].as_str().unwrap();
            assert_eq!(is_asset_id(id), case["valid"].as_bool().unwrap(), "{id:?}");
        }
    }

    #[test]
    fn imported_asset_wire_keys_are_frozen() {
        let f = fixture();
        let asset = ImportedAsset {
            id: "a1b2c3d4e5f60718.gif".to_owned(),
            width: 498,
            height: 280,
            bytes: 1_834_221,
            animated: true,
            warning: None,
        };
        let v = serde_json::to_value(&asset).unwrap();
        assert_eq!(v, f["importedAssetSample"]);
        let mut keys: Vec<Value> = v
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .map(Value::from)
            .collect();
        keys.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        assert_eq!(Value::from(keys), f["importedAssetKeys"]);
    }

    /// The scheme name and the origin the overlay loads pictures from.
    #[test]
    fn the_charm_origin() {
        let f = fixture();
        assert_eq!(
            f["charmOrigin"],
            format!("http://{CHARM_SCHEME}.localhost/")
        );
    }

    #[test]
    fn assets_dir_name_table() {
        assert_eq!(assets_dir_name(false), "assets");
        assert_eq!(assets_dir_name(true), "assets-dev");
    }

    // ---- Files ----

    /// A fresh folder under the system temp folder, removed when dropped.
    /// The pictures go in `overlay/assets` inside it, as in the app.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Self {
            let root = std::env::temp_dir()
                .join(format!("divora-charm-test-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&root);
            fs::create_dir_all(&root).unwrap();
            Self(root)
        }

        fn assets(&self) -> PathBuf {
            self.0.join("overlay").join("assets")
        }

        fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, bytes).unwrap();
            path
        }

        /// Import temp files left in `overlay/` or `overlay/assets/`.
        fn temps(&self) -> Vec<PathBuf> {
            [self.0.join("overlay"), self.assets()]
                .iter()
                .filter_map(|d| fs::read_dir(d).ok())
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.to_string_lossy().ends_with(".part"))
                .collect()
        }

        /// The names in `overlay/assets/`.
        fn stored(&self) -> Vec<String> {
            fs::read_dir(self.assets()).map_or_else(
                |_| Vec::new(),
                |d| {
                    d.flatten()
                        .map(|e| e.file_name().to_string_lossy().into_owned())
                        .collect()
                },
            )
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn import_writes_once_and_is_idempotent() {
        let s = Scratch::new("idempotent");
        let picture = gif(
            (498, 280),
            Some(1),
            &[],
            &[[0, 0, 498, 280], [0, 0, 498, 280]],
        );
        let src = s.write("dance.gif", &picture);

        let first = import(&s.assets(), &src).unwrap();
        assert_eq!(
            first,
            ImportedAsset {
                id: asset_id(&picture, Kind::Gif),
                width: 498,
                height: 280,
                bytes: picture.len() as u64,
                animated: true,
                warning: None,
            }
        );
        let dest = s.assets().join(&first.id);
        assert_eq!(fs::read(&dest).unwrap(), picture);
        assert_eq!(s.temps(), Vec::<PathBuf>::new());

        // Age the stored copy, so a rewrite would show.
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        File::options()
            .write(true)
            .open(&dest)
            .unwrap()
            .set_modified(old)
            .unwrap();
        let second = import(&s.assets(), &src).unwrap();
        assert_eq!(second, first);
        assert_eq!(fs::metadata(&dest).unwrap().modified().unwrap(), old);
        assert_eq!(s.stored(), vec![first.id]);
        assert_eq!(s.temps(), Vec::<PathBuf>::new());
    }

    #[test]
    fn import_names_by_content_and_sniffed_type() {
        let s = Scratch::new("names");
        // The extension comes from the bytes, not the file's name.
        let still = png(40, 30);
        let named_gif = import(&s.assets(), &s.write("looks-like.gif", &still)).unwrap();
        assert_eq!(named_gif.id, asset_id(&still, Kind::Png));
        let photo = jpeg(0xC0, 8, 640, 480, &[], Some(&tiff(false, 6)));
        let jpg = import(&s.assets(), &s.write("photo.jpeg", &photo)).unwrap();
        assert_eq!(jpg.id, asset_id(&photo, Kind::Jpeg));
        assert_eq!((jpg.width, jpg.height, jpg.animated), (480, 640, false));
        // An APNG is stored as a PNG, and is animated.
        let apng = png_file(64, 32, &[b"acTL", b"IDAT", b"IEND"]);
        let spin = import(&s.assets(), &s.write("spin.apng", &apng)).unwrap();
        assert_eq!(spin.id, asset_id(&apng, Kind::Png));
        assert!(spin.animated);
        // The same bytes under two names are one picture.
        let again = import(&s.assets(), &s.write("again.png", &still)).unwrap();
        assert_eq!(again, named_gif);
        let mut stored = s.stored();
        stored.sort();
        let mut want = vec![named_gif.id, jpg.id, spin.id];
        want.sort();
        assert_eq!(stored, want);
    }

    #[test]
    fn import_repairs_a_damaged_asset() {
        let picture = webp_vp8x(300, 200, 0x02);
        let id = asset_id(&picture, Kind::Webp);
        let zeroed = vec![0; picture.len()];
        let half = picture[..picture.len() / 2].to_vec();
        let mut changed = picture.clone();
        *changed.last_mut().unwrap() ^= 0x01;
        for (name, damage) in [("zeroed", zeroed), ("half", half), ("changed", changed)] {
            let s = Scratch::new(&format!("repair-{name}"));
            fs::create_dir_all(s.assets()).unwrap();
            let dest = s.assets().join(&id);
            fs::write(&dest, &damage).unwrap();
            assert!(!same_contents(&dest, &picture), "{name}");

            let src = s.write("source.webp", &picture);
            let asset = import(&s.assets(), &src).unwrap();
            assert_eq!(asset.id, id, "{name}");
            assert_eq!(fs::read(&dest).unwrap(), picture, "{name}");
            assert!(s.temps().is_empty(), "{name}");
        }
    }

    #[test]
    fn same_contents_table() {
        let s = Scratch::new("same");
        let bytes: Vec<u8> = (0..200_000_u32)
            .map(|i| u8::try_from(i % 251).unwrap())
            .collect();
        let path = s.write("x", &bytes);
        assert!(same_contents(&path, &bytes));
        let mut other = bytes.clone();
        other[150_000] ^= 1;
        assert!(!same_contents(&path, &other));
        assert!(!same_contents(&path, &bytes[..199_999]));
        assert!(!same_contents(&s.0.join("missing"), &bytes));
        assert!(!same_contents(&s.0, &bytes));
        let empty = s.write("empty", b"");
        assert!(same_contents(&empty, b""));
    }

    #[test]
    fn import_refuses_dir_empty_too_large() {
        let s = Scratch::new("refuses");
        assert_eq!(import(&s.assets(), &s.0), Err(ImportError::NotAFile));
        let empty = s.write("empty.png", b"");
        assert_eq!(import(&s.assets(), &empty), Err(ImportError::NotAFile));
        let missing = s.0.join("missing.png");
        assert!(matches!(
            import(&s.assets(), &missing),
            Err(ImportError::Read(_))
        ));
        // One byte over the cap, and exactly at it (read, then refused as
        // no picture).
        let big = s.0.join("big.png");
        File::create(&big).unwrap().set_len(MAX_BYTES + 1).unwrap();
        assert_eq!(import(&s.assets(), &big), Err(ImportError::TooLarge));
        let at_cap = s.0.join("at-cap.png");
        File::create(&at_cap).unwrap().set_len(MAX_BYTES).unwrap();
        assert_eq!(import(&s.assets(), &at_cap), Err(ImportError::NotAPicture));
        let svg = s.write("vector.svg", b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>");
        assert_eq!(import(&s.assets(), &svg), Err(ImportError::NotAPicture));
        // Nothing was written.
        assert!(!s.assets().exists());
        assert_eq!(s.temps(), Vec::<PathBuf>::new());
    }

    #[test]
    fn import_leaves_no_temp_on_failure() {
        let picture = png(16, 16);
        let id = asset_id(&picture, Kind::Png);

        // The rename fails: a folder sits under the picture's name.
        let s = Scratch::new("no-temp-rename");
        fs::create_dir_all(s.assets().join(&id)).unwrap();
        let src = s.write("p.png", &picture);
        assert!(matches!(
            import(&s.assets(), &src),
            Err(ImportError::Write(_))
        ));
        assert_eq!(s.temps(), Vec::<PathBuf>::new());

        // The pictures' folder can't be made: a file is in the way.
        let s = Scratch::new("no-temp-dir");
        fs::create_dir_all(s.0.join("overlay")).unwrap();
        fs::write(s.assets(), b"in the way").unwrap();
        let src = s.write("p.png", &picture);
        assert!(matches!(
            import(&s.assets(), &src),
            Err(ImportError::Write(_))
        ));
        assert_eq!(s.temps(), Vec::<PathBuf>::new());

        // A refused picture writes nothing at all.
        let s = Scratch::new("no-temp-refused");
        let src = s.write("big.png", &png(9000, 9000));
        assert_eq!(import(&s.assets(), &src), Err(ImportError::TooManyPixels));
        assert_eq!(s.temps(), Vec::<PathBuf>::new());
        assert!(!s.0.join("overlay").exists());
    }

    #[test]
    fn delete_validates_and_tolerates_missing() {
        let s = Scratch::new("delete");
        let dir = s.assets();
        fs::create_dir_all(&dir).unwrap();
        let picture = png(2, 2);
        let id = asset_id(&picture, Kind::Png);
        fs::write(dir.join(&id), &picture).unwrap();

        assert_eq!(delete(&dir, &id), Ok(()));
        assert!(!dir.join(&id).exists());
        // Already gone: fine.
        assert_eq!(delete(&dir, &id), Ok(()));

        // Nothing but an id names a file, so nothing outside is touched.
        let outside = s.write("x.png", &picture);
        let upper = id.to_uppercase();
        let spaced = format!("{id} ");
        for bad in [
            "../x.png",
            "x.png",
            "",
            "..",
            upper.as_str(),
            spaced.as_str(),
        ] {
            assert_eq!(
                delete(&dir, bad),
                Err("That isn't a charm picture.".to_owned()),
                "{bad:?}"
            );
        }
        assert!(outside.exists());

        // Any other failure is reported.
        let folder = asset_id(b"a folder", Kind::Gif);
        fs::create_dir(dir.join(&folder)).unwrap();
        let err = delete(&dir, &folder).unwrap_err();
        assert!(err.starts_with("Couldn't remove the picture: "), "{err}");
    }

    // ---- Serving ----

    fn header(r: &Response<Vec<u8>>, name: HeaderName) -> Option<&str> {
        r.headers().get(name).and_then(|v| v.to_str().ok())
    }

    /// Every row of the scheme's table.
    #[test]
    #[allow(clippy::too_many_lines)] // every row of the scheme's table
    fn serve_statuses_and_headers() {
        let s = Scratch::new("serve");
        let dir = s.assets();
        fs::create_dir_all(&dir).unwrap();
        let store = |name: &str, bytes: &[u8]| {
            fs::write(dir.join(name), bytes).unwrap();
            format!("/{name}")
        };
        let get = |path: &str| serve(Some(&dir), "overlay", "GET", path, || true);
        let refused = |r: Response<Vec<u8>>, status: StatusCode, why: &str| {
            assert_eq!(r.status(), status, "{why}");
            assert!(r.body().is_empty(), "{why}");
            assert_eq!(header(&r, CONTENT_TYPE), None, "{why}");
        };

        // Each kind, with its headers.
        let pictures = [
            (png(2, 1), Kind::Png, "image/png"),
            (jpeg(0xC2, 8, 3, 2, &[], None), Kind::Jpeg, "image/jpeg"),
            (
                gif((4, 4), None, &[], &[[0, 0, 4, 4]]),
                Kind::Gif,
                "image/gif",
            ),
            (webp_vp8l(5, 5), Kind::Webp, "image/webp"),
        ];
        for (bytes, kind, content_type) in &pictures {
            let path = store(&asset_id(bytes, *kind), bytes);
            let r = get(&path);
            assert_eq!(r.status(), StatusCode::OK, "{kind:?}");
            assert_eq!(r.body(), bytes, "{kind:?}");
            assert_eq!(header(&r, CONTENT_TYPE), Some(*content_type));
            assert_eq!(
                header(&r, CACHE_CONTROL),
                Some("public, max-age=31536000, immutable")
            );
            assert_eq!(header(&r, X_CONTENT_TYPE_OPTIONS), Some("nosniff"));
        }
        let ok = format!("/{}", asset_id(&pictures[0].0, Kind::Png));

        // No assets dir yet: the app is still starting.
        refused(
            serve(None, "overlay", "GET", &ok, || true),
            StatusCode::SERVICE_UNAVAILABLE,
            "no dir",
        );
        // Only the overlay loads pictures.
        for webview in ["main", "", "overlay-2", "Overlay"] {
            refused(
                serve(Some(&dir), webview, "GET", &ok, || true),
                StatusCode::FORBIDDEN,
                webview,
            );
        }
        // GET only.
        for method in ["POST", "HEAD", "PUT", "DELETE", "get"] {
            let r = serve(Some(&dir), "overlay", method, &ok, || true);
            assert_eq!(header(&r, ALLOW), Some("GET"), "{method}");
            refused(r, StatusCode::METHOD_NOT_ALLOWED, method);
        }
        // Only an id names a file.
        let bare = ok.trim_start_matches('/').to_owned();
        let upper = ok.to_uppercase();
        let jpeg_ext = ok.replace(".png", ".jpeg");
        let slash = format!("{ok}/");
        for path in [
            "/",
            "",
            bare.as_str(),
            upper.as_str(),
            jpeg_ext.as_str(),
            slash.as_str(),
            "/../x.png",
            "/a/b.png",
            "/..\\0123456789abcdef.png",
        ] {
            refused(get(path), StatusCode::NOT_FOUND, path);
        }
        // A valid id with no file.
        refused(
            get("/0123456789abcdef.gif"),
            StatusCode::NOT_FOUND,
            "missing",
        );
        // A read error: a folder under an id's name.
        let folder = asset_id(b"a folder", Kind::Png);
        fs::create_dir(dir.join(&folder)).unwrap();
        refused(
            get(&format!("/{folder}")),
            StatusCode::INTERNAL_SERVER_ERROR,
            "read error",
        );
        // Over the cap on disk: a real PNG header, so only the cap refuses it.
        let huge = asset_id(b"huge", Kind::Png);
        fs::write(dir.join(&huge), png(1, 1)).unwrap();
        File::options()
            .write(true)
            .open(dir.join(&huge))
            .unwrap()
            .set_len(MAX_BYTES + 1)
            .unwrap();
        refused(
            get(&format!("/{huge}")),
            StatusCode::NOT_FOUND,
            "over the cap",
        );
        // The bytes must be what the id's extension says.
        let photo = jpeg(0xC0, 8, 3, 2, &[], None);
        let as_png = format!("{}.png", &asset_id(&photo, Kind::Jpeg)[..16]);
        let path = store(&as_png, &photo);
        refused(get(&path), StatusCode::NOT_FOUND, "a jpeg named .png");
        let path = store("00000000000000ff.gif", b"<svg/>");
        refused(get(&path), StatusCode::NOT_FOUND, "an svg named .gif");
        // A closing overlay gets nothing, and is asked only after the read.
        refused(
            serve(Some(&dir), "overlay", "GET", &ok, || false),
            StatusCode::SERVICE_UNAVAILABLE,
            "not live",
        );
        refused(
            serve(
                Some(&dir),
                "overlay",
                "GET",
                "/0123456789abcdef.gif",
                || false,
            ),
            StatusCode::NOT_FOUND,
            "missing, not live",
        );
    }

    #[test]
    fn serve_gate_admits_two() {
        let gate = ServeGate::new(SERVE_SLOTS);
        assert_eq!(SERVE_SLOTS, 2);

        // Two places at once, without waiting; a third waits for one back.
        let first = gate.acquire();
        let second = gate.acquire();
        let (tx, rx) = mpsc::channel();
        thread::scope(|scope| {
            let g = &gate;
            scope.spawn(move || {
                let _third = g.acquire();
                tx.send(()).unwrap();
            });
            assert!(
                rx.recv_timeout(Duration::from_millis(150)).is_err(),
                "a third place while two are held"
            );
            drop(first);
            rx.recv_timeout(Duration::from_secs(10))
                .expect("the third gets the place given back");
        });
        drop(second);

        // Many threads at once: never more than two hold a place.
        let holding = AtomicUsize::new(0);
        let most = AtomicUsize::new(0);
        thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..200 {
                        let _slot = gate.acquire();
                        let now = holding.fetch_add(1, Ordering::SeqCst) + 1;
                        most.fetch_max(now, Ordering::SeqCst);
                        thread::yield_now();
                        holding.fetch_sub(1, Ordering::SeqCst);
                    }
                });
            }
        });
        assert!(most.load(Ordering::SeqCst) <= 2);
        assert_eq!(holding.load(Ordering::SeqCst), 0);
        // Every place came back.
        let _a = gate.acquire();
        let _b = gate.acquire();
    }
}
