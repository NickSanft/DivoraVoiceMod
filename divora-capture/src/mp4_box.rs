//! A small MP4 box reader for the smoke tests and the gate (v1.54.0). Not part of the API.
//!
//! The direct path writes MP4s through Media Foundation's sink with a
//! hand-built H.264 media type, and whether a file is right is decided by
//! boxes no player shows: which samples `stss` marks as keyframes (it is the
//! seek index), whether an edit list or composition offsets crept in, what
//! `avcC` and `colr` say, how long each track's `mdhd` says it is. The smoke
//! tests, the gate and `examples/mp4_inspect.rs` all read those through
//! [`parse`], so they agree on what a file holds.
//!
//! It reads only what those checks need, of the boxes a non-fragmented MP4
//! has: `ftyp`, `moov` → `mvhd` and each `trak` → `tkhd`, `edts`/`elst`,
//! `mdia` → `mdhd`, `hdlr`, `minf`/`stbl` → `stsd` (the first entry: its
//! size and `avcC`/`colr`, or its channels and rate), `stts`, `ctts`, `stss`,
//! `stsz`, `stsc` and `stco` or `co64`. From the last three it works out
//! where every sample is, and checks each lies inside the file.
//!
//! The input is whatever is on disk, a half-written `.part` included, so
//! every size and count is checked before it is used: a short or malformed
//! file is an [`Mp4Error`], never a panic, and a count can't make it
//! allocate more than the file could hold.

use std::fmt;
use std::ops::Range;

/// A box type: four bytes, usually ASCII.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct FourCc(pub [u8; 4]);

impl PartialEq<[u8; 4]> for FourCc {
    fn eq(&self, other: &[u8; 4]) -> bool {
        self.0 == *other
    }
}

impl fmt::Display for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for &b in &self.0 {
            if b.is_ascii_graphic() || b == b' ' {
                write!(f, "{}", char::from(b))?;
            } else {
                write!(f, "\\x{b:02x}")?;
            }
        }
        Ok(())
    }
}

impl fmt::Debug for FourCc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "'{self}'")
    }
}

/// Why a file could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mp4Error {
    /// A box or a field runs past the bytes there are: the file was cut
    /// short, or a size in it is wrong. `at` is the file offset it starts at.
    Truncated { what: &'static str, at: u64 },
    /// A box whose size is smaller than its own header.
    BadSize { kind: FourCc, at: u64, size: u64 },
    /// A box the file must have is not there (`moov` in a file that was
    /// never finalized, for one).
    Missing { what: &'static str },
    /// Tables that contradict each other or the file: a sample past the end
    /// of the file, chunks that hold fewer samples than there are, a version
    /// this reader doesn't know.
    Invalid { what: &'static str, at: u64 },
}

impl fmt::Display for Mp4Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { what, at } => write!(f, "{what} at byte {at} runs past the end"),
            Self::BadSize { kind, at, size } => {
                write!(f, "box {kind} at byte {at} has an impossible size {size}")
            }
            Self::Missing { what } => write!(f, "no {what} box"),
            Self::Invalid { what, at } => write!(f, "{what} (at byte {at})"),
        }
    }
}

impl std::error::Error for Mp4Error {}

/// Where a top-level box is in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoxSpan {
    pub kind: FourCc,
    /// File offset of the box's first byte.
    pub offset: u64,
    /// The whole box, header included.
    pub size: u64,
    /// 8, or 16 for a 64-bit size (or more for a `uuid` box).
    pub header: u64,
}

/// The `ftyp` box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ftyp {
    pub major: FourCc,
    pub minor: u32,
    pub compatible: Vec<FourCc>,
}

/// One edit-list entry. Times are in the movie's timescale (`mvhd`) for the
/// duration and the track's (`mdhd`) for `media_time`, which is −1 for an
/// empty edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Edit {
    pub segment_duration: u64,
    pub media_time: i64,
    pub rate: i16,
}

/// The H.264 decoder configuration (`avcC`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AvcConfig {
    pub profile: u8,
    pub compatibility: u8,
    pub level: u8,
    /// Bytes in each sample's NAL length prefix (1, 2 or 4).
    pub length_size: u8,
    /// Each sequence parameter set, without a start code or length.
    pub sps: Vec<Vec<u8>>,
    /// Each picture parameter set, the same way.
    pub pps: Vec<Vec<u8>>,
}

/// The colour box (`colr`). The numbers are ISO/IEC 23091-2 code points
/// (1/1/1 is BT.709); a `colr` of another kind (an ICC profile) leaves them 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Colr {
    /// `nclx`, `nclc` (Apple's older form, with no range flag), `prof` or
    /// `rICC`.
    pub kind: FourCc,
    pub primaries: u16,
    pub transfer: u16,
    pub matrix: u16,
    /// Full range (0–255) rather than limited (16–235).
    pub full_range: bool,
}

/// What the first sample description says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SampleEntry {
    /// A video track's entry (`avc1` for H.264).
    Video {
        kind: FourCc,
        width: u16,
        height: u16,
        avcc: Option<AvcConfig>,
        colr: Option<Colr>,
    },
    /// A sound track's entry (`mp4a` for AAC). The rate is the entry's
    /// 16.16 field, which holds rates up to 65 535 Hz.
    Audio {
        kind: FourCc,
        channels: u16,
        sample_rate: u32,
    },
    /// Any other handler's entry, by type only.
    Other { kind: FourCc },
}

/// One `stsc` entry: from chunk `first_chunk` (1-based) on, each chunk holds
/// `samples_per_chunk` samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRun {
    pub first_chunk: u32,
    pub samples_per_chunk: u32,
    pub description: u32,
}

/// One track.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub track_id: u32,
    /// `vide` or `soun` (`hdlr`).
    pub handler: FourCc,
    /// Units per second of the track's times (`mdhd`).
    pub timescale: u32,
    /// The track's length in its timescale (`mdhd`).
    pub duration: u64,
    /// 0 for 32-bit times, 1 for 64-bit.
    pub mdhd_version: u8,
    /// `None` without an `edts` box; an `edts` without an `elst` is `Some`
    /// of nothing.
    pub edits: Option<Vec<Edit>>,
    /// The first sample description, and how many there are.
    pub entry: SampleEntry,
    pub entry_count: u32,
    /// `(sample count, delta)` runs (`stts`).
    pub stts: Vec<(u32, u32)>,
    /// `(sample count, offset)` runs, or `None` without a `ctts` box.
    pub ctts: Option<Vec<(u32, i64)>>,
    /// The sync samples, 1-based (`stss`). `None` — no `stss` box — means
    /// every sample is a sync sample; `Some` of nothing means none is. The
    /// two must never be confused: see [`Track::is_sync`].
    pub stss: Option<Vec<u32>>,
    /// Every sample's size (`stsz`, expanded).
    pub sizes: Vec<u32>,
    pub chunk_runs: Vec<ChunkRun>,
    /// Every chunk's file offset (`stco`, or `co64`).
    pub chunk_offsets: Vec<u64>,
    /// Whether the chunk offsets came from `co64`.
    pub co64: bool,
    /// Every sample's file offset, worked out from the three tables above.
    pub offsets: Vec<u64>,
}

impl Track {
    pub fn is_video(&self) -> bool {
        self.handler == *b"vide"
    }

    pub fn is_audio(&self) -> bool {
        self.handler == *b"soun"
    }

    pub fn sample_count(&self) -> usize {
        self.sizes.len()
    }

    /// Whether 1-based sample `n` is a sync sample. Without an `stss` box
    /// every sample is one; with an empty one, none is.
    pub fn is_sync(&self, n: u32) -> bool {
        self.stss
            .as_ref()
            .is_none_or(|s| s.binary_search(&n).is_ok())
    }

    /// Where 0-based sample `i` is in the file.
    pub fn sample_range(&self, i: usize) -> Option<Range<u64>> {
        let start = *self.offsets.get(i)?;
        let size = *self.sizes.get(i)?;
        Some(start..start.checked_add(u64::from(size))?)
    }

    /// Sample `i`'s bytes, from the file this track was parsed from.
    pub fn sample<'f>(&self, file: &'f [u8], i: usize) -> Option<&'f [u8]> {
        let r = self.sample_range(i)?;
        file.get(usize::try_from(r.start).ok()?..usize::try_from(r.end).ok()?)
    }

    pub fn avcc(&self) -> Option<&AvcConfig> {
        match &self.entry {
            SampleEntry::Video { avcc, .. } => avcc.as_ref(),
            _ => None,
        }
    }

    pub fn colr(&self) -> Option<Colr> {
        match &self.entry {
            SampleEntry::Video { colr, .. } => *colr,
            _ => None,
        }
    }
}

/// What [`parse`] read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Mp4 {
    /// The top-level boxes, in file order.
    pub top: Vec<BoxSpan>,
    pub ftyp: Option<Ftyp>,
    /// The movie's timescale and duration (`mvhd`).
    pub timescale: u32,
    pub duration: u64,
    /// The tracks, in file order.
    pub tracks: Vec<Track>,
}

impl Mp4 {
    /// The first video track.
    pub fn video(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.is_video())
    }

    /// The first sound track.
    pub fn audio(&self) -> Option<&Track> {
        self.tracks.iter().find(|t| t.is_audio())
    }
}

/// Reads an MP4 held in memory. Errs on anything cut short or inconsistent
/// (see the module docs); a file with no `moov` — never finalized — is
/// [`Mp4Error::Missing`].
pub fn parse(file: &[u8]) -> Result<Mp4, Mp4Error> {
    let file_len = wide(file.len());
    let top = boxes(file, 0)?;
    let spans = top
        .iter()
        .map(|b| BoxSpan {
            kind: b.kind,
            offset: b.at,
            size: b.size,
            header: b.size.saturating_sub(wide(b.body.len())),
        })
        .collect();
    let ftyp = find(&top, *b"ftyp").map(parse_ftyp).transpose()?;
    let moov = find(&top, *b"moov").ok_or(Mp4Error::Missing { what: "moov" })?;
    let children = boxes(moov.body, moov.body_at)?;
    let mvhd = require(&children, *b"mvhd", "mvhd")?;
    let (timescale, duration) = parse_times(mvhd, "mvhd")?;
    let tracks = children
        .iter()
        .filter(|b| b.kind == *b"trak")
        .map(|trak| parse_trak(trak, file_len))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Mp4 {
        top: spans,
        ftyp,
        timescale,
        duration,
        tracks,
    })
}

/// The NAL unit types in one AVCC-framed sample (each NAL preceded by its
/// length in `length_size` bytes), in order. Errs unless the lengths use up
/// the sample exactly: a sample still in Annex B form (start codes), or one
/// cut short, does not.
pub fn avcc_nal_types(sample: &[u8], length_size: u8) -> Result<Vec<u8>, Mp4Error> {
    if !matches!(length_size, 1 | 2 | 4) {
        return Err(Mp4Error::Invalid {
            what: "an AVCC length size other than 1, 2 or 4",
            at: 0,
        });
    }
    let mut r = Reader::new(sample, 0, "an AVCC NAL unit");
    let mut types = Vec::new();
    while r.remaining() > 0 {
        let len = match length_size {
            1 => u32::from(r.u8()?),
            2 => u32::from(r.u16()?),
            _ => r.u32()?,
        };
        let nal = r.take(usize::try_from(len).unwrap_or(usize::MAX))?;
        let Some(&header) = nal.first() else {
            return Err(Mp4Error::Invalid {
                what: "an empty AVCC NAL unit",
                at: r.at(),
            });
        };
        types.push(header & 0x1F);
    }
    Ok(types)
}

/// A box inside some bytes: its type, where it is and its payload.
#[derive(Clone, Copy)]
struct RawBox<'a> {
    kind: FourCc,
    /// File offset of the header.
    at: u64,
    /// The whole box, header included.
    size: u64,
    body: &'a [u8],
    /// File offset of the payload.
    body_at: u64,
}

/// The boxes that fill `bytes` exactly, which start at file offset `base`.
/// A size of 0 runs to the end of `bytes`; a size of 1 is followed by a
/// 64-bit size.
fn boxes(bytes: &[u8], base: u64) -> Result<Vec<RawBox<'_>>, Mp4Error> {
    let mut out = Vec::new();
    let mut rest = bytes;
    let mut at = base;
    while !rest.is_empty() {
        let mut r = Reader::new(rest, at, "a box header");
        let size32 = r.u32()?;
        let kind = FourCc(r.array4()?);
        let size = match size32 {
            0 => wide(rest.len()),
            1 => r.u64()?,
            n => u64::from(n),
        };
        if kind == *b"uuid" {
            r.take(16)?;
        }
        let header = wide(r.pos);
        if size < header {
            return Err(Mp4Error::BadSize { kind, at, size });
        }
        let end = usize::try_from(size)
            .ok()
            .filter(|&end| end <= rest.len())
            .ok_or(Mp4Error::Truncated { what: "a box", at })?;
        let body = rest.get(r.pos..end).unwrap_or_default();
        out.push(RawBox {
            kind,
            at,
            size,
            body,
            body_at: at + header,
        });
        rest = rest.get(end..).unwrap_or_default();
        at += size;
    }
    Ok(out)
}

fn find<'a, 'b>(siblings: &'b [RawBox<'a>], kind: [u8; 4]) -> Option<&'b RawBox<'a>> {
    siblings.iter().find(|b| b.kind == kind)
}

fn require<'a, 'b>(
    siblings: &'b [RawBox<'a>],
    kind: [u8; 4],
    what: &'static str,
) -> Result<&'b RawBox<'a>, Mp4Error> {
    find(siblings, kind).ok_or(Mp4Error::Missing { what })
}

/// The children of the box `kind` among `siblings`.
fn children_of<'a>(
    siblings: &[RawBox<'a>],
    kind: [u8; 4],
    what: &'static str,
) -> Result<Vec<RawBox<'a>>, Mp4Error> {
    let b = require(siblings, kind, what)?;
    boxes(b.body, b.body_at)
}

/// A full box's payload after its version and flags, and the version.
fn full_box<'a>(b: &RawBox<'a>, what: &'static str) -> Result<(u8, Reader<'a>), Mp4Error> {
    let mut r = Reader::new(b.body, b.body_at, what);
    let version = r.u8()?;
    r.take(3)?;
    Ok((version, r))
}

fn unknown_version(what: &'static str, b: &RawBox<'_>) -> Mp4Error {
    Mp4Error::Invalid { what, at: b.at }
}

fn parse_ftyp(b: &RawBox<'_>) -> Result<Ftyp, Mp4Error> {
    let mut r = Reader::new(b.body, b.body_at, "ftyp");
    let major = FourCc(r.array4()?);
    let minor = r.u32()?;
    let mut compatible = Vec::new();
    while r.remaining() >= 4 {
        compatible.push(FourCc(r.array4()?));
    }
    Ok(Ftyp {
        major,
        minor,
        compatible,
    })
}

/// `mvhd` and `mdhd` start alike: created, modified, timescale, duration,
/// 32-bit in version 0 and 64-bit (but the timescale) in version 1.
fn parse_times(b: &RawBox<'_>, what: &'static str) -> Result<(u32, u64), Mp4Error> {
    let (version, mut r) = full_box(b, what)?;
    match version {
        0 => {
            r.take(8)?;
            Ok((r.u32()?, u64::from(r.u32()?)))
        }
        1 => {
            r.take(16)?;
            Ok((r.u32()?, r.u64()?))
        }
        _ => Err(unknown_version(
            "an mvhd or mdhd version other than 0 or 1",
            b,
        )),
    }
}

fn parse_trak(trak: &RawBox<'_>, file_len: u64) -> Result<Track, Mp4Error> {
    let children = boxes(trak.body, trak.body_at)?;
    let tkhd = require(&children, *b"tkhd", "tkhd")?;
    let (version, mut r) = full_box(tkhd, "tkhd")?;
    r.take(if version == 1 { 16 } else { 8 })?;
    let track_id = r.u32()?;
    let edits = find(&children, *b"edts").map(parse_edits).transpose()?;

    let mdia = children_of(&children, *b"mdia", "mdia")?;
    let mdhd = require(&mdia, *b"mdhd", "mdhd")?;
    let (timescale, duration) = parse_times(mdhd, "mdhd")?;
    let mdhd_version = full_box(mdhd, "mdhd")?.0;
    let (_, mut r) = full_box(require(&mdia, *b"hdlr", "hdlr")?, "hdlr")?;
    r.take(4)?;
    let handler = FourCc(r.array4()?);
    let minf = children_of(&mdia, *b"minf", "minf")?;
    let stbl = children_of(&minf, *b"stbl", "stbl")?;

    let (entry, entry_count) = parse_stsd(require(&stbl, *b"stsd", "stsd")?, handler)?;
    let deltas = parse_stts(require(&stbl, *b"stts", "stts")?)?;
    let ctts = find(&stbl, *b"ctts").map(parse_ctts).transpose()?;
    let sync = find(&stbl, *b"stss").map(parse_stss).transpose()?;
    let sizes = parse_stsz(require(&stbl, *b"stsz", "stsz")?, file_len)?;
    let chunk_runs = parse_stsc(require(&stbl, *b"stsc", "stsc")?)?;
    let (chunk_offsets, co64) = match (find(&stbl, *b"stco"), find(&stbl, *b"co64")) {
        (Some(stco), _) => (parse_chunk_offsets(stco, false)?, false),
        (None, Some(co64)) => (parse_chunk_offsets(co64, true)?, true),
        (None, None) => {
            return Err(Mp4Error::Missing {
                what: "stco or co64",
            })
        }
    };
    let offsets = sample_offsets(&chunk_runs, &chunk_offsets, &sizes, file_len, trak.at)?;
    Ok(Track {
        track_id,
        handler,
        timescale,
        duration,
        mdhd_version,
        edits,
        entry,
        entry_count,
        stts: deltas,
        ctts,
        stss: sync,
        sizes,
        chunk_runs,
        chunk_offsets,
        co64,
        offsets,
    })
}

fn parse_edits(edts: &RawBox<'_>) -> Result<Vec<Edit>, Mp4Error> {
    let children = boxes(edts.body, edts.body_at)?;
    let Some(elst) = find(&children, *b"elst") else {
        return Ok(Vec::new());
    };
    let (version, mut r) = full_box(elst, "elst")?;
    let count = r.u32()?;
    let entry_len = match version {
        0 => 12,
        1 => 20,
        _ => return Err(unknown_version("an elst version other than 0 or 1", elst)),
    };
    table(&mut r, count, entry_len, |r| {
        let (segment_duration, media_time) = if version == 1 {
            (r.u64()?, r.i64()?)
        } else {
            (u64::from(r.u32()?), i64::from(r.i32()?))
        };
        let rate = r.i16()?;
        r.take(2)?;
        Ok(Edit {
            segment_duration,
            media_time,
            rate,
        })
    })
}

fn parse_stsd(stsd: &RawBox<'_>, handler: FourCc) -> Result<(SampleEntry, u32), Mp4Error> {
    let (_, mut r) = full_box(stsd, "stsd")?;
    let count = r.u32()?;
    let entries = boxes(r.rest(), r.at())?;
    let first = entries.first().ok_or(Mp4Error::Missing {
        what: "sample description",
    })?;
    let entry = if handler == *b"vide" {
        parse_visual(first)?
    } else if handler == *b"soun" {
        parse_audio(first)?
    } else {
        SampleEntry::Other { kind: first.kind }
    };
    Ok((entry, count))
}

/// A `VisualSampleEntry`: 78 bytes of fields, then child boxes.
fn parse_visual(entry: &RawBox<'_>) -> Result<SampleEntry, Mp4Error> {
    let mut r = Reader::new(entry.body, entry.body_at, "a video sample entry");
    // reserved (6), data reference index, pre_defined, reserved, pre_defined × 3
    r.take(24)?;
    let width = r.u16()?;
    let height = r.u16()?;
    // resolutions, reserved, frame count, compressor name (32), depth, pre_defined
    r.take(50)?;
    let children = boxes(r.rest(), r.at())?;
    let avcc = find(&children, *b"avcC").map(parse_avcc).transpose()?;
    let colr = find(&children, *b"colr").map(parse_colr).transpose()?;
    Ok(SampleEntry::Video {
        kind: entry.kind,
        width,
        height,
        avcc,
        colr,
    })
}

/// An `AudioSampleEntry`'s fixed fields. Its children (`esds`) are not read.
fn parse_audio(entry: &RawBox<'_>) -> Result<SampleEntry, Mp4Error> {
    let mut r = Reader::new(entry.body, entry.body_at, "a sound sample entry");
    // reserved (6), data reference index, then 8 reserved (QuickTime's version, revision, vendor)
    r.take(16)?;
    let channels = r.u16()?;
    // sample size, pre_defined, reserved
    r.take(6)?;
    let sample_rate = r.u32()? >> 16;
    Ok(SampleEntry::Audio {
        kind: entry.kind,
        channels,
        sample_rate,
    })
}

fn parse_avcc(b: &RawBox<'_>) -> Result<AvcConfig, Mp4Error> {
    let mut r = Reader::new(b.body, b.body_at, "avcC");
    r.u8()?; // configuration version
    let profile = r.u8()?;
    let compatibility = r.u8()?;
    let level = r.u8()?;
    let length_size = (r.u8()? & 0x03) + 1;
    let sps_count = r.u8()? & 0x1F;
    let mut sps = Vec::new();
    for _ in 0..sps_count {
        let len = r.u16()?;
        sps.push(r.take(usize::from(len))?.to_vec());
    }
    let pps_count = r.u8()?;
    let mut pps = Vec::new();
    for _ in 0..pps_count {
        let len = r.u16()?;
        pps.push(r.take(usize::from(len))?.to_vec());
    }
    Ok(AvcConfig {
        profile,
        compatibility,
        level,
        length_size,
        sps,
        pps,
    })
}

fn parse_colr(b: &RawBox<'_>) -> Result<Colr, Mp4Error> {
    let mut r = Reader::new(b.body, b.body_at, "colr");
    let kind = FourCc(r.array4()?);
    let mut colr = Colr {
        kind,
        primaries: 0,
        transfer: 0,
        matrix: 0,
        full_range: false,
    };
    if kind == *b"nclx" || kind == *b"nclc" {
        colr.primaries = r.u16()?;
        colr.transfer = r.u16()?;
        colr.matrix = r.u16()?;
        if kind == *b"nclx" {
            colr.full_range = r.u8()? & 0x80 != 0;
        }
    }
    Ok(colr)
}

fn parse_stts(b: &RawBox<'_>) -> Result<Vec<(u32, u32)>, Mp4Error> {
    let (_, mut r) = full_box(b, "stts")?;
    let count = r.u32()?;
    table(&mut r, count, 8, |r| Ok((r.u32()?, r.u32()?)))
}

/// Version 0 offsets are unsigned, version 1 signed.
fn parse_ctts(b: &RawBox<'_>) -> Result<Vec<(u32, i64)>, Mp4Error> {
    let (version, mut r) = full_box(b, "ctts")?;
    let count = r.u32()?;
    table(&mut r, count, 8, |r| {
        let n = r.u32()?;
        let offset = if version == 0 {
            i64::from(r.u32()?)
        } else {
            i64::from(r.i32()?)
        };
        Ok((n, offset))
    })
}

fn parse_stss(b: &RawBox<'_>) -> Result<Vec<u32>, Mp4Error> {
    let (_, mut r) = full_box(b, "stss")?;
    let count = r.u32()?;
    table(&mut r, count, 4, Reader::u32)
}

/// Every sample's size. A box with one size for all expands it, but only as
/// far as the file could hold that many samples.
fn parse_stsz(b: &RawBox<'_>, file_len: u64) -> Result<Vec<u32>, Mp4Error> {
    let (_, mut r) = full_box(b, "stsz")?;
    let size = r.u32()?;
    let count = r.u32()?;
    if size == 0 {
        return table(&mut r, count, 4, Reader::u32);
    }
    if u64::from(count) * u64::from(size) > file_len {
        return Err(Mp4Error::Invalid {
            what: "more samples than the file holds",
            at: b.at,
        });
    }
    Ok(vec![size; usize::try_from(count).unwrap_or_default()])
}

fn parse_stsc(b: &RawBox<'_>) -> Result<Vec<ChunkRun>, Mp4Error> {
    let (_, mut r) = full_box(b, "stsc")?;
    let count = r.u32()?;
    table(&mut r, count, 12, |r| {
        Ok(ChunkRun {
            first_chunk: r.u32()?,
            samples_per_chunk: r.u32()?,
            description: r.u32()?,
        })
    })
}

fn parse_chunk_offsets(b: &RawBox<'_>, wide_offsets: bool) -> Result<Vec<u64>, Mp4Error> {
    let what = if wide_offsets { "co64" } else { "stco" };
    let (_, mut r) = full_box(b, what)?;
    let count = r.u32()?;
    if wide_offsets {
        table(&mut r, count, 8, Reader::u64)
    } else {
        table(&mut r, count, 4, |r| r.u32().map(u64::from))
    }
}

/// Where each sample is: chunk by chunk, each chunk's samples back to back
/// from its offset, as many per chunk as the `stsc` run in force says. Every
/// sample must lie inside the file and in some chunk.
fn sample_offsets(
    runs: &[ChunkRun],
    chunks: &[u64],
    sizes: &[u32],
    file_len: u64,
    at: u64,
) -> Result<Vec<u64>, Mp4Error> {
    let invalid = |what| Mp4Error::Invalid { what, at };
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut run = 0;
    for (chunk, &chunk_offset) in (1u64..).zip(chunks) {
        if offsets.len() == sizes.len() {
            break;
        }
        while runs
            .get(run + 1)
            .is_some_and(|next| u64::from(next.first_chunk) <= chunk)
        {
            run += 1;
        }
        let per_chunk = runs
            .get(run)
            .filter(|r| u64::from(r.first_chunk) <= chunk)
            .ok_or(invalid("a chunk no stsc entry covers"))?
            .samples_per_chunk;
        let mut pos = chunk_offset;
        for _ in 0..per_chunk {
            let Some(&size) = sizes.get(offsets.len()) else {
                break;
            };
            let end = pos
                .checked_add(u64::from(size))
                .filter(|&end| end <= file_len)
                .ok_or(invalid("a sample past the end of the file"))?;
            offsets.push(pos);
            pos = end;
        }
    }
    if offsets.len() < sizes.len() {
        return Err(invalid("samples that no chunk holds"));
    }
    Ok(offsets)
}

/// `count` entries of `entry_len` bytes each, read by `read` — checked
/// against the bytes left first, so a wild count can't allocate.
fn table<'a, T>(
    r: &mut Reader<'a>,
    count: u32,
    entry_len: usize,
    mut read: impl FnMut(&mut Reader<'a>) -> Result<T, Mp4Error>,
) -> Result<Vec<T>, Mp4Error> {
    let count = usize::try_from(count).unwrap_or(usize::MAX);
    if count
        .checked_mul(entry_len)
        .is_none_or(|n| n > r.remaining())
    {
        return Err(Mp4Error::Truncated {
            what: r.what,
            at: r.at(),
        });
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(read(r)?);
    }
    Ok(out)
}

/// A file offset or length from a slice length.
fn wide(n: usize) -> u64 {
    u64::try_from(n).unwrap_or(u64::MAX)
}

/// Big-endian fields from a slice, each read checked.
struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    /// File offset of `bytes[0]`, for errors.
    base: u64,
    /// What is being read, for errors.
    what: &'static str,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], base: u64, what: &'static str) -> Self {
        Self {
            bytes,
            pos: 0,
            base,
            what,
        }
    }

    /// The file offset of the next byte.
    fn at(&self) -> u64 {
        self.base + wide(self.pos)
    }

    fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    fn rest(&self) -> &'a [u8] {
        self.bytes.get(self.pos..).unwrap_or_default()
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], Mp4Error> {
        let out = self
            .pos
            .checked_add(n)
            .and_then(|end| self.bytes.get(self.pos..end))
            .ok_or(Mp4Error::Truncated {
                what: self.what,
                at: self.at(),
            })?;
        self.pos += n;
        Ok(out)
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N], Mp4Error> {
        let mut out = [0; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    fn array4(&mut self) -> Result<[u8; 4], Mp4Error> {
        self.array()
    }

    fn u8(&mut self) -> Result<u8, Mp4Error> {
        Ok(self.array::<1>()?[0])
    }

    fn u16(&mut self) -> Result<u16, Mp4Error> {
        self.array().map(u16::from_be_bytes)
    }

    fn i16(&mut self) -> Result<i16, Mp4Error> {
        self.array().map(i16::from_be_bytes)
    }

    fn u32(&mut self) -> Result<u32, Mp4Error> {
        self.array().map(u32::from_be_bytes)
    }

    fn i32(&mut self) -> Result<i32, Mp4Error> {
        self.array().map(i32::from_be_bytes)
    }

    fn u64(&mut self) -> Result<u64, Mp4Error> {
        self.array().map(u64::from_be_bytes)
    }

    fn i64(&mut self) -> Result<i64, Mp4Error> {
        self.array().map(i64::from_be_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box: 32-bit size, type, payload.
    fn bx(kind: &[u8], body: &[u8]) -> Vec<u8> {
        let size = u32::try_from(8 + body.len()).unwrap();
        [&size.to_be_bytes()[..], kind, body].concat()
    }

    /// A full box: version, no flags, payload.
    fn full(kind: &[u8], version: u8, body: &[u8]) -> Vec<u8> {
        bx(kind, &[&[version, 0, 0, 0][..], body].concat())
    }

    fn be32(values: &[u32]) -> Vec<u8> {
        values.iter().flat_map(|v| v.to_be_bytes()).collect()
    }

    /// How the `mdat` box's size is written.
    #[derive(Clone, Copy)]
    enum MdatSize {
        Exact,
        /// 0: to the end of the file.
        Zero,
        /// 1, then a 64-bit size.
        Large,
    }

    #[derive(Clone)]
    struct Spec {
        mdhd_v1: bool,
        stss: Option<Vec<u32>>,
        co64: bool,
        mdat: MdatSize,
    }

    impl Default for Spec {
        fn default() -> Self {
            Self {
                mdhd_v1: false,
                stss: Some(vec![1]),
                co64: false,
                mdat: MdatSize::Exact,
            }
        }
    }

    const SPS: [u8; 4] = [0x67, 0x64, 0x00, 0x0D];
    const PPS: [u8; 4] = [0x68, 0xEE, 0x3C, 0x80];

    /// Three AVCC-framed video samples (an IDR, then two P-frames) and one
    /// audio sample.
    fn samples() -> [Vec<u8>; 4] {
        [
            [&be32(&[2])[..], &[0x65, 0x88]].concat(),
            [&be32(&[2])[..], &[0x41, 0x9A]].concat(),
            [&be32(&[3])[..], &[0x41, 0x9B, 0x01]].concat(),
            vec![0x21, 0x10, 0x04, 0x60],
        ]
    }

    fn avc1() -> Vec<u8> {
        let mut fields = vec![0; 24];
        fields.extend(320u16.to_be_bytes());
        fields.extend(240u16.to_be_bytes());
        fields.extend([0; 50]);
        let avcc = [
            &[1, 0x64, 0x00, 0x0D, 0xFF, 0xE1][..],
            &4u16.to_be_bytes(),
            &SPS,
            &[1],
            &4u16.to_be_bytes(),
            &PPS,
        ]
        .concat();
        let colr = [&b"nclx"[..], &[0, 1, 0, 1, 0, 1, 0x00]].concat();
        [fields, bx(b"avcC", &avcc), bx(b"colr", &colr)].concat()
    }

    fn mp4a() -> Vec<u8> {
        let mut fields = vec![0; 16];
        fields.extend(2u16.to_be_bytes());
        fields.extend(16u16.to_be_bytes());
        fields.extend([0; 4]);
        fields.extend((48_000u32 << 16).to_be_bytes());
        fields
    }

    /// One track whose chunks start at `chunks` (file offsets), holding
    /// `runs` of samples of `sizes`.
    fn trak(
        spec: &Spec,
        handler: &[u8],
        entry: (&[u8], &[u8]),
        sizes: &[u32],
        runs: &[[u32; 3]],
        chunks: &[u64],
        sync: Option<&[u32]>,
    ) -> Vec<u8> {
        let tkhd = full(b"tkhd", 0, &be32(&[0, 0, 1, 0, 0]));
        let mdhd = if spec.mdhd_v1 {
            let mut b = vec![0; 16];
            b.extend(90_000u32.to_be_bytes());
            b.extend(((1u64 << 33) + 7).to_be_bytes());
            full(b"mdhd", 1, &[b, vec![0; 4]].concat())
        } else {
            full(b"mdhd", 0, &be32(&[0, 0, 30_000, 3_000, 0]))
        };
        let hdlr = full(b"hdlr", 0, &[&[0; 4][..], handler, &[0; 13]].concat());
        let description = full(b"stsd", 0, &[be32(&[1]), bx(entry.0, entry.1)].concat());
        let n = u32::try_from(sizes.len()).unwrap();
        let mut stbl = [
            description,
            full(b"stts", 0, &be32(&[1, n, 1_000])),
            full(b"stsz", 0, &[be32(&[0, n]), be32(sizes)].concat()),
            full(
                b"stsc",
                0,
                &[
                    be32(&[u32::try_from(runs.len()).unwrap()]),
                    be32(&runs.concat()),
                ]
                .concat(),
            ),
        ]
        .concat();
        if let Some(s) = sync {
            let count = u32::try_from(s.len()).unwrap();
            stbl.extend(full(b"stss", 0, &[be32(&[count]), be32(s)].concat()));
        }
        let count = be32(&[u32::try_from(chunks.len()).unwrap()]);
        stbl.extend(if spec.co64 {
            let offsets: Vec<u8> = chunks.iter().flat_map(|c| c.to_be_bytes()).collect();
            full(b"co64", 0, &[count, offsets].concat())
        } else {
            let offsets: Vec<u32> = chunks.iter().map(|&c| u32::try_from(c).unwrap()).collect();
            full(b"stco", 0, &[count, be32(&offsets)].concat())
        });
        let minf = bx(b"minf", &bx(b"stbl", &stbl));
        bx(
            b"trak",
            &[tkhd, bx(b"mdia", &[mdhd, hdlr, minf].concat())].concat(),
        )
    }

    /// `ftyp`, `moov` (a video track and an audio track), `mdat`. The video
    /// is in two chunks — samples 1–2, then 3 — and the second chunk is
    /// written FIRST, so only the chunk offsets say where anything is.
    fn file(spec: &Spec) -> Vec<u8> {
        let [s1, s2, s3, a1] = samples();
        let payload = [s3.clone(), s1.clone(), s2.clone(), a1.clone()].concat();
        let ftyp = bx(
            b"ftyp",
            &[&b"isom"[..], &be32(&[512]), b"isomavc1"].concat(),
        );
        let moov_at = |base: u64| {
            let sizes = |s: &[&Vec<u8>]| -> Vec<u32> {
                s.iter().map(|x| u32::try_from(x.len()).unwrap()).collect()
            };
            let len = |x: &Vec<u8>| u64::try_from(x.len()).unwrap();
            let video = trak(
                spec,
                b"vide",
                (b"avc1", &avc1()),
                &sizes(&[&s1, &s2, &s3]),
                &[[1, 2, 1], [2, 1, 1]],
                &[base + len(&s3), base],
                spec.stss.as_deref(),
            );
            let audio = trak(
                spec,
                b"soun",
                (b"mp4a", &mp4a()),
                &sizes(&[&a1]),
                &[[1, 1, 1]],
                &[base + len(&s3) + len(&s1) + len(&s2)],
                None,
            );
            bx(
                b"moov",
                &[full(b"mvhd", 0, &be32(&[0, 0, 1_000, 100])), video, audio].concat(),
            )
        };
        let header_len = if matches!(spec.mdat, MdatSize::Large) {
            16
        } else {
            8
        };
        let base = u64::try_from(ftyp.len() + moov_at(0).len() + header_len).unwrap();
        let moov = moov_at(base);
        let mdat_size = u64::try_from(header_len + payload.len()).unwrap();
        let mdat = match spec.mdat {
            MdatSize::Exact => bx(b"mdat", &payload),
            MdatSize::Zero => [&be32(&[0])[..], b"mdat", &payload].concat(),
            MdatSize::Large => {
                [&be32(&[1])[..], b"mdat", &mdat_size.to_be_bytes(), &payload].concat()
            }
        };
        [ftyp, moov, mdat].concat()
    }

    /// The video samples come back where they were written, AVCC-framed.
    fn assert_samples(m: &Mp4, f: &[u8]) {
        let v = m.video().unwrap();
        let [s1, s2, s3, a1] = samples();
        assert_eq!(v.sample(f, 0), Some(&s1[..]));
        assert_eq!(v.sample(f, 1), Some(&s2[..]));
        assert_eq!(v.sample(f, 2), Some(&s3[..]));
        assert_eq!(v.sample(f, 3), None);
        assert_eq!(m.audio().unwrap().sample(f, 0), Some(&a1[..]));
        let length = v.avcc().unwrap().length_size;
        assert_eq!(avcc_nal_types(v.sample(f, 0).unwrap(), length), Ok(vec![5]));
        assert_eq!(avcc_nal_types(v.sample(f, 2).unwrap(), length), Ok(vec![1]));
    }

    #[test]
    fn a_size_zero_box_runs_to_the_end_of_the_file() {
        let bytes = file(&Spec {
            mdat: MdatSize::Zero,
            ..Spec::default()
        });
        let mp4 = parse(&bytes).unwrap();
        let kinds: Vec<FourCc> = mp4.top.iter().map(|b| b.kind).collect();
        assert_eq!(
            kinds,
            [FourCc(*b"ftyp"), FourCc(*b"moov"), FourCc(*b"mdat")]
        );
        let mdat = mp4.top[2];
        assert_eq!(mdat.offset + mdat.size, u64::try_from(bytes.len()).unwrap());
        assert_eq!(mdat.header, 8);
        assert_samples(&mp4, &bytes);

        // Everything else a test reads, from the same file.
        let ftyp = mp4.ftyp.as_ref().unwrap();
        assert_eq!((ftyp.major, ftyp.minor), (FourCc(*b"isom"), 512));
        assert_eq!(ftyp.compatible, [FourCc(*b"isom"), FourCc(*b"avc1")]);
        assert_eq!((mp4.timescale, mp4.duration), (1_000, 100));
        let video = mp4.video().unwrap();
        assert_eq!(video.entry_count, 1);
        assert!(matches!(
            video.entry,
            SampleEntry::Video {
                width: 320,
                height: 240,
                ..
            }
        ));
        let avcc = video.avcc().unwrap();
        assert_eq!(
            (avcc.profile, avcc.level, avcc.length_size),
            (0x64, 0x0D, 4)
        );
        assert_eq!(
            (avcc.sps.as_slice(), avcc.pps.as_slice()),
            (&[SPS.to_vec()][..], &[PPS.to_vec()][..])
        );
        let colr = video.colr().unwrap();
        assert_eq!(
            (
                colr.kind,
                colr.primaries,
                colr.transfer,
                colr.matrix,
                colr.full_range
            ),
            (FourCc(*b"nclx"), 1, 1, 1, false)
        );
        assert_eq!(
            (video.timescale, video.duration, video.mdhd_version),
            (30_000, 3_000, 0)
        );
        assert_eq!(video.stts, [(3, 1_000)]);
        assert_eq!((video.ctts.as_ref(), video.edits.as_ref()), (None, None));
        let audio = mp4.audio().unwrap();
        assert_eq!(
            audio.entry,
            SampleEntry::Audio {
                kind: FourCc(*b"mp4a"),
                channels: 2,
                sample_rate: 48_000
            }
        );
    }

    #[test]
    fn a_size_one_box_reads_its_64_bit_size() {
        let f = file(&Spec {
            mdat: MdatSize::Large,
            ..Spec::default()
        });
        let m = parse(&f).unwrap();
        let mdat = m.top[2];
        assert_eq!(mdat.kind, *b"mdat");
        assert_eq!(mdat.header, 16);
        assert_eq!(mdat.offset + mdat.size, u64::try_from(f.len()).unwrap());
        assert_samples(&m, &f);

        // A 64-bit size smaller than its own 16-byte header is no box.
        let bad = [&be32(&[1])[..], b"free", &8u64.to_be_bytes()].concat();
        assert_eq!(
            parse(&bad),
            Err(Mp4Error::BadSize {
                kind: FourCc(*b"free"),
                at: 0,
                size: 8
            })
        );
    }

    #[test]
    fn a_truncated_file_is_an_error_never_a_panic() {
        let f = file(&Spec::default());
        assert_samples(&parse(&f).unwrap(), &f);
        // Cut anywhere — mid-header, mid-table, between boxes, mid-sample —
        // and the file is refused.
        for n in 0..f.len() {
            assert!(parse(&f[..n]).is_err(), "cut at {n} of {}", f.len());
        }

        // Sizes that lie: shorter than a header, longer than the parent.
        let tiny = [&be32(&[4])[..], b"free"].concat();
        assert!(matches!(
            parse(&tiny),
            Err(Mp4Error::BadSize { size: 4, .. })
        ));
        let long = bx(b"moov", &[&be32(&[64])[..], b"mvhd"].concat());
        assert!(matches!(
            parse(&long),
            Err(Mp4Error::Truncated { at: 8, .. })
        ));
        // A count far beyond the box is refused before anything is allocated.
        let stts = full(b"stts", 0, &be32(&[u32::MAX, 1, 1]));
        let r = RawBox {
            kind: FourCc(*b"stts"),
            at: 0,
            size: u64::try_from(stts.len()).unwrap(),
            body: &stts[8..],
            body_at: 8,
        };
        assert!(matches!(parse_stts(&r), Err(Mp4Error::Truncated { .. })));

        // AVCC framing: a length past the sample, a stray byte, and Annex B
        // start codes are all refused; the real thing is consumed exactly.
        let [s1, ..] = samples();
        assert_eq!(avcc_nal_types(&s1, 4), Ok(vec![5]));
        assert!(avcc_nal_types(&s1[..s1.len() - 1], 4).is_err());
        assert!(avcc_nal_types(&[&s1[..], &[0]].concat(), 4).is_err());
        assert!(avcc_nal_types(&[0, 0, 0, 1, 0x65, 0x88], 4).is_err());
    }

    #[test]
    fn mdhd_version_1_has_64_bit_times() {
        let f = file(&Spec {
            mdhd_v1: true,
            ..Spec::default()
        });
        let m = parse(&f).unwrap();
        for t in &m.tracks {
            assert_eq!(t.mdhd_version, 1);
            assert_eq!(t.timescale, 90_000);
            assert_eq!(t.duration, (1 << 33) + 7);
        }
        assert_samples(&m, &f);
    }

    #[test]
    fn an_absent_stss_is_not_an_empty_one() {
        let parsed = |stss: Option<Vec<u32>>| {
            let m = parse(&file(&Spec {
                stss,
                ..Spec::default()
            }))
            .unwrap();
            m.video().unwrap().clone()
        };
        // No stss box: every sample is a sync sample.
        let absent = parsed(None);
        assert_eq!(absent.stss, None);
        assert!((1..=3).all(|n| absent.is_sync(n)));
        // An empty one: none is.
        let empty = parsed(Some(vec![]));
        assert_eq!(empty.stss, Some(vec![]));
        assert!((1..=3).all(|n| !empty.is_sync(n)));
        // And a list says which.
        let listed = parsed(Some(vec![1, 3]));
        assert_eq!(listed.stss, Some(vec![1, 3]));
        assert_eq!(
            (1..=3).map(|n| listed.is_sync(n)).collect::<Vec<_>>(),
            [true, false, true]
        );
    }

    #[test]
    fn co64_offsets_are_read_like_stco() {
        let narrow = parse(&file(&Spec::default())).unwrap();
        let bytes = file(&Spec {
            co64: true,
            ..Spec::default()
        });
        let mp4 = parse(&bytes).unwrap();
        let (wide, narrow_video) = (mp4.video().unwrap(), narrow.video().unwrap());
        assert!(wide.co64 && !narrow_video.co64);
        // The second chunk is first in the mdat; the first chunk follows it.
        let mdat = mp4.top[2];
        assert_eq!(wide.chunk_offsets[1], mdat.offset + mdat.header);
        assert_eq!(
            wide.chunk_offsets[0] - wide.chunk_offsets[1],
            narrow_video.chunk_offsets[0] - narrow_video.chunk_offsets[1]
        );
        assert_samples(&mp4, &bytes);

        // A 64-bit offset past the end of the file is refused, not read.
        let past = [&be32(&[1])[..], &(1u64 << 40).to_be_bytes()].concat();
        let co64 = full(b"co64", 0, &past);
        let raw = RawBox {
            kind: FourCc(*b"co64"),
            at: 0,
            size: u64::try_from(co64.len()).unwrap(),
            body: &co64[8..],
            body_at: 8,
        };
        let chunks = parse_chunk_offsets(&raw, true).unwrap();
        assert_eq!(chunks, [1u64 << 40]);
        let runs = [ChunkRun {
            first_chunk: 1,
            samples_per_chunk: 1,
            description: 1,
        }];
        assert!(sample_offsets(&runs, &chunks, &[4], 1 << 20, 0).is_err());
    }
}
