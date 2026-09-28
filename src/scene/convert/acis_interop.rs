//! ACIS that other programs can read back.
//!
//! WHY THIS MODULE EXISTS
//!
//! Every solid this engine wrote was unreadable outside of it. Measured on
//! 2026-09-28 against the production server binary (`moteur-latest`,
//! 7378ad15) under WSL, relayed through ODA File Converter 27.1 with its audit
//! switched OFF (the audit "repairs" an unreadable solid by dropping it, and
//! says nothing): BOX, CYLINDER, CONE, SPHERE, TORUS, WEDGE, PYRAMID, EXTRUDE,
//! REVOLVE, UNION, SUBTRACT — and all 369 solids of the production drawing
//! CAD-000018 — came back as `Object improperly read: AcDb3dSolid`, then
//! `Empty ACIS not allowed` or `Out of memory`. The engine reads its own files
//! without complaint, which is why nothing inside it ever noticed.
//!
//! The DWG container is not at fault: fed a valid SAB body (ezdxf's
//! TrueView-tested exporter), the same writer produces a DWG that ODA reads,
//! for R2018 (AcDs data store) and R2010 (inline SAT) alike. The fault is the
//! ACIS the geometry kernel emits (`cadkernel::acis::append`, starting from
//! acadrust's `SatHeader::new`). Compared record by record with ezdxf's
//! exporter and with the AutoCAD region in acadrust's examples, it omits
//! fields the ACIS 7.0 layout carries:
//!
//! * `edge`             — the trailing convexity string (`@7 unknown`);
//! * `straight-curve`   — its two parameter bounds (`I I`);
//! * `ellipse-curve`    — its two parameter bounds;
//! * `plane-surface`, `sphere-surface`, `torus-surface`
//!                      — the `forward_v` sense and four bounds;
//! * `cone-surface`     — the `forward` sense and four bounds.
//!
//! Measured one change at a time on the production SAB, those missing fields
//! alone decide whether ODA reads a solid. Four other departures from what
//! AutoCAD writes are corrected with them, because every reference reader
//! and writer agrees on them and ODA reads them either way:
//!
//! * the header counts ZERO bodies (ezdxf documents the body count as
//!   required by Autodesk products) and lists the body LAST, where ACIS
//!   writes the saved entities first;
//! * the header declares 10 mm units where AutoCAD, ASM and acadrust's own
//!   SAB normalisation write 1;
//! * a straight curve's direction is the edge's segment vector with edge
//!   parameters 0..1, where ACIS stores a UNIT direction and measures edge
//!   parameters as lengths. A reader that normalises the direction would
//!   place every straight edge's end at one drawing unit from its start.
//!
//! Only documents the kernel wrote are touched: acadrust's product id and an
//! ACIS 7 header, and within them only records whose tokens have exactly the
//! shape the kernel emits. A solid read from AutoCAD, or already conformed,
//! comes back untouched.
//!
//! The second half of the module prepares a document for SAVING: it applies
//! the same conformity to solids a drawing already carries (every solid
//! written before this fix), and it removes the solid histories whose root
//! claims the solid itself as the object it owns — see
//! [`prepare_for_interchange`].

use std::collections::{HashMap, HashSet};

use acadrust::entities::acis::types::{SatDocument, SatPointer, SatRecord, SatToken};
use acadrust::entities::acis::{SabReader, SabWriter};
use acadrust::entities::AcisData;
use acadrust::objects::{DynamicBlockData, ObjectType};
use acadrust::{CadDocument, EntityType, Handle};

/// The product id acadrust's `SatHeader::new` stamps on every document the
/// kernel fills. It is what marks a document as ours.
pub const KERNEL_PRODUCT: &str = "acadrust";
/// The unit scale (mm per drawing unit) acadrust's `SatHeader::new` writes.
const KERNEL_UNITS: f64 = 10.0;
/// What AutoCAD, ASM, ezdxf and acadrust's own SAB normalisation write.
const ACIS_UNITS: f64 = 1.0;
/// acadrust's version string for a 7.0 document.
const KERNEL_PRODUCT_VERSION: &str = "ACIS 7.0";
/// The 7.0 version string ezdxf found Autodesk products to accept.
const ACIS_700_PRODUCT_VERSION: &str = "ACIS 32.0 NT";

/// True when the document was filled by the kernel through acadrust: its
/// header carries acadrust's product id and an ACIS 7 version.
pub fn is_kernel_document(document: &SatDocument) -> bool {
    document.header.product_id == KERNEL_PRODUCT && document.header.version.major == 7
}

/// Brings a kernel-written document up to the ACIS 7.0 layout other readers
/// require. Returns whether anything changed. Idempotent, and a no-op on any
/// document [`is_kernel_document`] does not recognise.
pub fn conform_kernel_sat(document: &mut SatDocument) -> bool {
    if !is_kernel_document(document) {
        return false;
    }
    // Directions first: the pass recognises a straight curve by its kernel
    // shape, which `complete_records` then extends.
    let mut changed = unit_line_directions(document);
    changed |= complete_records(document);
    changed |= bodies_first(document);
    changed |= complete_header(document);
    changed
}

// ---------------------------------------------------------------------------
// Token shapes
// ---------------------------------------------------------------------------

/// One run of a record's tokens, as far as its layout is concerned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Run {
    /// A pointer (`$n`).
    Pointer,
    /// Consecutive numbers: `n` values, however they are encoded (SAT text
    /// floats and integers, SAB doubles, SAB positions and directions).
    Numbers(usize),
    /// Anything else: a keyword, a boolean, a string.
    Word,
}

/// How many numbers a token carries, or `None` when it is not a number.
fn number_count(token: &SatToken) -> Option<usize> {
    if let Some((_, count)) = token.coordinate_components() {
        return Some(count);
    }
    match token {
        SatToken::Float(_) | SatToken::Integer(_) => Some(1),
        SatToken::Sab {
            tag: 0x02 | 0x03 | 0x04 | 0x05 | 0x06 | 0x17,
            ..
        } => Some(1),
        _ => None,
    }
}

/// The record's tokens folded into runs: consecutive numbers merge, so a SAT
/// text position (three floats) and a SAB position (one token) read alike.
fn shape(tokens: &[SatToken]) -> Vec<Run> {
    let mut runs: Vec<Run> = Vec::with_capacity(tokens.len());
    for token in tokens {
        let run = if token.as_pointer().is_some() {
            Run::Pointer
        } else if let Some(count) = number_count(token) {
            if let Some(Run::Numbers(previous)) = runs.last_mut() {
                *previous += count;
                continue;
            }
            Run::Numbers(count)
        } else {
            Run::Word
        };
        runs.push(run);
    }
    runs
}

/// Is this token an edge or coedge sense (`forward` / `reversed`, or the SAB
/// booleans a reader may have left undecoded)?
fn is_sense(token: &SatToken) -> bool {
    matches!(token, SatToken::True | SatToken::False)
        || matches!(token.as_ident(), Some("forward" | "reversed"))
}

/// Every number a record carries, flattened in order.
fn numbers(tokens: &[SatToken]) -> Vec<f64> {
    let mut out = Vec::new();
    for token in tokens {
        if let Some((components, count)) = token.coordinate_components() {
            out.extend_from_slice(&components[..count]);
        } else if number_count(token).is_some() {
            out.push(token.as_float().unwrap_or(f64::NAN));
        }
    }
    out
}

/// Overwrites the numbers from flattened slot `first` on with `values`,
/// keeping each token's own encoding: a SAT float stays a float, a SAB double
/// stays a SAB double, a SAB direction stays a SAB direction. Integers become
/// floats (SAT text writes `10` for the double 10.0). Returns false, having
/// changed nothing, when a slot sits in a token that cannot hold a real.
fn set_numbers(tokens: &mut [SatToken], first: usize, values: &[f64]) -> bool {
    let wanted = first..first + values.len();
    // Pass 1: every token the slots touch must be able to hold a real.
    let mut slot = 0usize;
    for token in tokens.iter() {
        let Some(count) = number_count(token) else {
            continue;
        };
        let covered = slot..slot + count;
        let touched = covered.start < wanted.end && wanted.start < covered.end;
        let writable = matches!(
            token,
            SatToken::Float(_)
                | SatToken::Integer(_)
                | SatToken::Position(..)
                | SatToken::Sab { tag: 0x06 | 0x13 | 0x14, .. }
        );
        if touched && !writable {
            return false;
        }
        slot += count;
    }
    if slot < wanted.end {
        return false;
    }
    // Pass 2: write.
    let mut slot = 0usize;
    for token in tokens.iter_mut() {
        let Some(count) = number_count(token) else {
            continue;
        };
        let value_at = |offset: usize| -> Option<f64> {
            let at = slot + offset;
            wanted.contains(&at).then(|| values[at - first])
        };
        match token {
            SatToken::Float(_) | SatToken::Integer(_) => {
                if let Some(value) = value_at(0) {
                    *token = SatToken::Float(value);
                }
            }
            SatToken::Position(x, y, z) => {
                for (offset, component) in [x, y, z].into_iter().enumerate() {
                    if let Some(value) = value_at(offset) {
                        *component = value;
                    }
                }
            }
            SatToken::Sab { tag: 0x06, data } => {
                if let Some(value) = value_at(0) {
                    *data = value.to_le_bytes().to_vec();
                }
            }
            SatToken::Sab {
                tag: 0x13 | 0x14,
                data,
            } if data.len() == 24 => {
                for offset in 0..3 {
                    if let Some(value) = value_at(offset) {
                        data[offset * 8..offset * 8 + 8].copy_from_slice(&value.to_le_bytes());
                    }
                }
            }
            _ => {}
        }
        slot += count;
    }
    true
}

// ---------------------------------------------------------------------------
// The four corrections
// ---------------------------------------------------------------------------

/// A kernel straight curve: its pattern pointer, then a root point and a
/// direction — six numbers and nothing else.
fn is_kernel_straight(record: &SatRecord) -> bool {
    record.entity_type == "straight-curve"
        && shape(&record.tokens) == [Run::Pointer, Run::Numbers(6)]
}

/// Scales every kernel straight curve's direction to a unit vector, and the
/// parameters of the edges that run along it by the same length, so every
/// edge still starts and ends where it did.
fn unit_line_directions(document: &mut SatDocument) -> bool {
    let mut lengths: HashMap<i32, f64> = HashMap::new();
    for (position, record) in document.records.iter_mut().enumerate() {
        if !is_kernel_straight(record) {
            continue;
        }
        let values = numbers(&record.tokens);
        let direction = [values[3], values[4], values[5]];
        let length = direction.iter().map(|c| c * c).sum::<f64>().sqrt();
        if !length.is_finite() || length <= 0.0 || (length - 1.0).abs() <= 1e-12 {
            continue;
        }
        let unit = direction.map(|c| c / length);
        if set_numbers(&mut record.tokens, 3, &unit) {
            record.raw_text = None;
            lengths.insert(position as i32, length);
        }
    }
    if lengths.is_empty() {
        return false;
    }
    for record in document.records.iter_mut() {
        if record.entity_type != "edge" {
            continue;
        }
        // `$pattern $start start_param $end end_param $coedge $curve sense ...`
        let Some(curve) = record.token_pointer(6) else {
            continue;
        };
        let Some(&length) = lengths.get(&curve.0) else {
            continue;
        };
        for index in [2usize, 4] {
            let Some(value) = record.tokens.get(index).and_then(SatToken::as_float) else {
                continue;
            };
            let scaled = value * length;
            let token = &mut record.tokens[index];
            match token {
                SatToken::Sab { tag: 0x06, data } => *data = scaled.to_le_bytes().to_vec(),
                SatToken::Float(_) | SatToken::Integer(_) => *token = SatToken::Float(scaled),
                _ => continue,
            }
        }
        record.raw_text = None;
    }
    true
}

/// Appends what the ACIS 7.0 layout carries after the fields the kernel
/// writes. Each rule fires only on the exact token shape the kernel emits, so
/// a complete record — from AutoCAD, or already conformed — never matches.
fn complete_records(document: &mut SatDocument) -> bool {
    const BOUNDS: &[&str] = &["I", "I"];
    const SURFACE_V: &[&str] = &["forward_v", "I", "I", "I", "I"];
    const SURFACE_U: &[&str] = &["forward", "I", "I", "I", "I"];
    let mut changed = false;
    for record in document.records.iter_mut() {
        let runs = shape(&record.tokens);
        let tail: &[&str] = match (record.entity_type.as_str(), runs.as_slice()) {
            ("straight-curve", [Run::Pointer, Run::Numbers(6)]) => BOUNDS,
            ("ellipse-curve", [Run::Pointer, Run::Numbers(10)]) => BOUNDS,
            ("plane-surface", [Run::Pointer, Run::Numbers(9)]) => SURFACE_V,
            ("sphere-surface", [Run::Pointer, Run::Numbers(10)]) => SURFACE_V,
            ("torus-surface", [Run::Pointer, Run::Numbers(11)]) => SURFACE_V,
            (
                "cone-surface",
                [Run::Pointer, Run::Numbers(10), Run::Word, Run::Word, Run::Numbers(3)],
            ) => SURFACE_U,
            (
                "edge",
                [
                    Run::Pointer,
                    Run::Pointer,
                    Run::Numbers(1),
                    Run::Pointer,
                    Run::Numbers(1),
                    Run::Pointer,
                    Run::Pointer,
                    Run::Word,
                ],
            ) if record.tokens.last().is_some_and(is_sense) => {
                // A counted string, not a keyword: `@7 unknown` in SAT text, a
                // 0x07 string in SAB.
                record.tokens.push(SatToken::String("unknown".to_string()));
                record.raw_text = None;
                changed = true;
                continue;
            }
            _ => continue,
        };
        record
            .tokens
            .extend(tail.iter().map(|word| SatToken::Ident((*word).to_string())));
        record.raw_text = None;
        changed = true;
    }
    changed
}

/// Moves the body records to the front, as ACIS writes the saved entities
/// first, and renumbers every pointer to follow. Every other record keeps its
/// relative order, so the file-global numbering of subtype blocks (`{ ref n }`)
/// is unchanged: a body record carries none, and a document whose body did
/// would be left as it is.
fn bodies_first(document: &mut SatDocument) -> bool {
    let count = document.records.len();
    let bodies: Vec<usize> = (0..count)
        .filter(|&index| document.records[index].entity_type == "body")
        .collect();
    if bodies.iter().enumerate().all(|(order, &index)| order == index) {
        return false;
    }
    if bodies.iter().any(|&index| {
        document.records[index]
            .tokens
            .iter()
            .any(|token| token.as_ident() == Some("{"))
    }) {
        return false;
    }
    let is_end = |record: &SatRecord| record.entity_type.starts_with("End-of");
    let mut order = bodies.clone();
    order.extend((0..count).filter(|&index| {
        document.records[index].entity_type != "body" && !is_end(&document.records[index])
    }));
    order.extend((0..count).filter(|&index| is_end(&document.records[index])));
    let mut moved_to = vec![0i32; count];
    for (new, &old) in order.iter().enumerate() {
        moved_to[old] = new as i32;
    }
    let remap = |pointer: SatPointer| -> SatPointer {
        match pointer.index() {
            Some(index) if index < count => SatPointer::new(moved_to[index]),
            _ => pointer,
        }
    };
    let mut slots: Vec<Option<SatRecord>> =
        std::mem::take(&mut document.records).into_iter().map(Some).collect();
    for (new, &old) in order.iter().enumerate() {
        let Some(mut record) = slots[old].take() else {
            continue;
        };
        record.index = new as i32;
        record.attribute = remap(record.attribute);
        for token in record.tokens.iter_mut() {
            if let SatToken::Pointer(pointer) = token {
                *pointer = remap(*pointer);
            }
        }
        record.raw_text = None;
        document.records.push(record);
    }
    true
}

/// The header AutoCAD writes for a 7.0 document: the body count, a unit scale
/// of one, and the version string Autodesk products are known to accept.
fn complete_header(document: &mut SatDocument) -> bool {
    let bodies = document
        .records
        .iter()
        .filter(|record| record.entity_type == "body")
        .count();
    let header = &mut document.header;
    let mut changed = false;
    if header.num_bodies != bodies {
        header.num_bodies = bodies;
        changed = true;
    }
    if header.spatial_resolution == KERNEL_UNITS {
        header.spatial_resolution = ACIS_UNITS;
        changed = true;
    }
    if header.product_version == KERNEL_PRODUCT_VERSION {
        header.product_version = ACIS_700_PRODUCT_VERSION.to_string();
        changed = true;
    }
    changed
}

// ---------------------------------------------------------------------------
// Entity ACIS payloads
// ---------------------------------------------------------------------------

/// What happened to one entity's ACIS payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AcisConformity {
    /// Not written by the kernel (or empty): left untouched.
    Foreign,
    /// Written by the kernel and already in the ACIS 7.0 layout.
    AlreadyConforming,
    /// Rewritten into the ACIS 7.0 layout.
    Conformed,
    /// Written by the kernel, but conforming it could not be proven safe —
    /// see the reason. The payload is left exactly as it was.
    Refused(&'static str),
}

/// The first line and product id of a SAT text payload, read without
/// parsing the records: `(version, bodies, product)`.
fn sat_text_header(sat: &str) -> Option<(u32, usize, &str)> {
    let mut lines = sat.lines();
    let mut first = lines.next()?.split_ascii_whitespace();
    let version = first.next()?.parse().ok()?;
    let _records: usize = first.next()?.parse().ok()?;
    let bodies = first.next()?.parse().ok()?;
    let mut second = lines.next()?.split_ascii_whitespace();
    let length: usize = second.next()?.trim_start_matches('@').parse().ok()?;
    let product = second.next()?;
    (product.len() == length).then_some((version, bodies, product))
}

/// The same three fields from a SAB payload's fixed header.
fn sab_header(sab: &[u8]) -> Option<(u32, usize, &str)> {
    const MAGIC: &[u8] = b"ACIS BinaryFile";
    if !sab.starts_with(MAGIC) {
        return None;
    }
    let word = |at: usize| -> Option<u32> {
        Some(u32::from_le_bytes(sab.get(at..at + 4)?.try_into().ok()?))
    };
    let version = word(15)?;
    let bodies = word(23)? as usize;
    // Then a 0x07 counted string: the product id.
    let tag = *sab.get(31)?;
    let length = *sab.get(32)? as usize;
    if tag != 0x07 {
        return None;
    }
    let product = std::str::from_utf8(sab.get(33..33 + length)?).ok()?;
    Some((version, bodies, product))
}

/// Does this payload carry the kernel's unconformed header? A cheap look at
/// the header only, so saving a drawing of thousands of conformed solids does
/// not parse any of them. The kernel writes ZERO bodies; the conformity pass
/// writes the count, so a conformed payload never matches again.
fn looks_like_unconformed_kernel_acis(acis: &AcisData) -> bool {
    let header = if acis.is_binary {
        sab_header(&acis.sab_data)
    } else {
        sat_text_header(&acis.sat_data)
    };
    matches!(header, Some((700..=799, 0, KERNEL_PRODUCT)))
}

/// What must hold for a rewrite to be accepted: the kernel lifts the same
/// number of bodies, with no more loss, the same number of faces, and the same
/// bounds.
#[derive(Debug, Clone, PartialEq)]
struct Lifted {
    bodies: usize,
    complete: bool,
    faces: usize,
    bounds: Option<([f64; 3], [f64; 3])>,
}

fn lifted(document: &SatDocument) -> Lifted {
    let (bodies, loss) = cadkernel::acis::lift(document);
    let mut bounds: Option<cadkernel::brep::Aabb> = None;
    let mut faces = 0usize;
    for body in &bodies {
        faces += body.face_keys().count();
        if let Some(each) = cadkernel::brep::body_bounds(body) {
            match bounds.as_mut() {
                Some(all) => all.merge(each),
                None => bounds = Some(each),
            }
        }
    }
    Lifted {
        bodies: bodies.len(),
        complete: loss.is_empty(),
        faces,
        bounds: bounds.map(|aabb| (aabb.min, aabb.max)),
    }
}

/// Same geometry within a relative tolerance on the bounds.
fn same_geometry(before: &Lifted, after: &Lifted) -> bool {
    if before.bodies != after.bodies
        || before.faces != after.faces
        || (before.complete && !after.complete)
    {
        return false;
    }
    match (before.bounds, after.bounds) {
        (None, None) => true,
        (Some((min_a, max_a)), Some((min_b, max_b))) => {
            let scale = min_a
                .iter()
                .chain(max_a.iter())
                .fold(1.0f64, |acc, value| acc.max(value.abs()));
            let tolerance = 1e-9 * scale;
            min_a
                .iter()
                .zip(min_b.iter())
                .chain(max_a.iter().zip(max_b.iter()))
                .all(|(a, b)| (a - b).abs() <= tolerance)
        }
        _ => false,
    }
}

/// Conforms one entity's ACIS payload in place — text or binary — when the
/// kernel wrote it and it predates the conformity.
///
/// A binary payload is only rewritten when acadrust's SAB reader and writer
/// reproduce it byte for byte BEFORE any change: the rewrite goes through the
/// same pair, so an encoding they do not round-trip is left alone rather than
/// risked. Both kinds are only replaced when the kernel lifts the rewrite into
/// the same geometry ([`same_geometry`]).
pub fn conform_acis_data(acis: &mut AcisData) -> AcisConformity {
    if !acis.has_data() {
        return AcisConformity::Foreign;
    }
    if !looks_like_unconformed_kernel_acis(acis) {
        let header = if acis.is_binary {
            sab_header(&acis.sab_data)
        } else {
            sat_text_header(&acis.sat_data)
        };
        return match header {
            Some((700..=799, _, KERNEL_PRODUCT)) => AcisConformity::AlreadyConforming,
            _ => AcisConformity::Foreign,
        };
    }
    if acis.is_binary {
        let Ok((mut document, consumed)) = SabReader::read_with_consumed(&acis.sab_data) else {
            return AcisConformity::Refused("SAB illisible");
        };
        if SabWriter::write(&document) != acis.sab_data[..consumed] {
            return AcisConformity::Refused("aller-retour SAB inexact");
        }
        let before = lifted(&document);
        if !conform_kernel_sat(&mut document) {
            return AcisConformity::AlreadyConforming;
        }
        let rewritten = SabWriter::write(&document);
        let Ok(reread) = SabReader::read(&rewritten) else {
            return AcisConformity::Refused("SAB conforme illisible");
        };
        if !same_geometry(&before, &lifted(&reread)) {
            return AcisConformity::Refused("geometrie changee");
        }
        acis.sab_data = rewritten;
        AcisConformity::Conformed
    } else {
        let Ok(mut document) = SatDocument::parse(&acis.sat_data) else {
            return AcisConformity::Refused("SAT illisible");
        };
        let before = lifted(&document);
        if !conform_kernel_sat(&mut document) {
            return AcisConformity::AlreadyConforming;
        }
        let text = document.to_sat_string();
        let Ok(reread) = SatDocument::parse(&text) else {
            return AcisConformity::Refused("SAT conforme illisible");
        };
        if !same_geometry(&before, &lifted(&reread)) {
            return AcisConformity::Refused("geometrie changee");
        }
        acis.sat_data = AcisData::strip_sat_terminator(&text);
        AcisConformity::Conformed
    }
}

// ---------------------------------------------------------------------------
// Saving
// ---------------------------------------------------------------------------

/// What [`prepare_for_interchange`] did to a document about to be written.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InterchangeReport {
    /// ACIS payloads rewritten into the 7.0 layout.
    pub acis_conformed: usize,
    /// Kernel payloads left as they were because the rewrite was not proven
    /// safe. They stay unreadable to ODA; the drawing still saves.
    pub acis_refused: usize,
    /// Self-owned solid histories removed.
    pub histories_removed: usize,
}

fn acis_of(entity: &EntityType) -> Option<&AcisData> {
    match entity {
        EntityType::Solid3D(value) => Some(&value.acis_data),
        EntityType::Region(value) => Some(&value.acis_data),
        EntityType::Body(value) => Some(&value.acis_data),
        EntityType::Surface(value) => Some(&value.acis_data),
        _ => None,
    }
}

fn acis_of_mut(entity: &mut EntityType) -> Option<&mut AcisData> {
    match entity {
        EntityType::Solid3D(value) => Some(&mut value.acis_data),
        EntityType::Region(value) => Some(&mut value.acis_data),
        EntityType::Body(value) => Some(&mut value.acis_data),
        EntityType::Surface(value) => Some(&mut value.acis_data),
        _ => None,
    }
}

fn history_of(entity: &EntityType) -> Option<Handle> {
    match entity {
        EntityType::Solid3D(value) => value.history_handle,
        EntityType::Region(value) => value.history_handle,
        EntityType::Body(value) => value.history_handle,
        EntityType::Surface(value) => value.history_handle,
        _ => None,
    }
    .filter(|handle| handle.is_valid())
}

fn clear_history(entity: &mut EntityType) {
    match entity {
        EntityType::Solid3D(value) => value.history_handle = None,
        EntityType::Region(value) => value.history_handle = None,
        EntityType::Body(value) => value.history_handle = None,
        EntityType::Surface(value) => value.history_handle = None,
        _ => {}
    }
}

/// Prepares a document SNAPSHOT for writing. Call it on the copy the writer
/// receives, never on the live document.
///
/// 1. Every kernel ACIS payload that predates the conformity is conformed
///    ([`conform_acis_data`]). This is what repairs a drawing saved by an
///    earlier engine: its solids come back from the file as the unconformed
///    SAB they were written as, and the next save rewrites them.
/// 2. Every solid history whose root names the solid itself as the object it
///    owns is left out, and the solid written without history — the form
///    AutoCAD writes with `SOLIDHIST` off. acadrust's `create_solid_history`
///    sets the root's owned-object handle (DXF 360) to the solid, where
///    AutoCAD's root owns an evaluation graph: the solid owns the root, which
///    owns the solid. Measured 2026-09-28, once the ACIS itself is readable:
///    * DXF: ODA File Converter dies of a stack overflow (exit 0xC00000FD) on
///      any such solid;
///    * DWG: a CONE, and the 3DROTATEd cylinders of the production drawing
///      CAD-000018 (1A6, 1A7, 1F1), still stop ODA with `Object improperly
///      read: AcDb3dSolid ... Out of memory` — the WHOLE drawing then fails to
///      open. The same solids written without their history read cleanly.
///    Nothing in the ERP's drawing flow uses a solid's history: a solid
///    reopened without one is lifted from its ACIS (`restore_solid_models`
///    already falls back to it). A history recorded by AutoCAD (whose root
///    owns something else) is always kept.
///
/// Only the entities concerned are copied out of their shared `Arc`s; the
/// header checks that select them read a few bytes each.
pub fn prepare_for_interchange(document: &mut CadDocument) -> InterchangeReport {
    let mut report = InterchangeReport::default();

    let unconformed: Vec<Handle> = document
        .entities()
        .filter(|entity| acis_of(entity).is_some_and(looks_like_unconformed_kernel_acis))
        .map(|entity| entity.common().handle)
        .collect();
    for handle in unconformed {
        let Some(acis) = document.get_entity_mut(handle).and_then(acis_of_mut) else {
            continue;
        };
        match conform_acis_data(acis) {
            AcisConformity::Conformed => report.acis_conformed += 1,
            AcisConformity::Refused(reason) => {
                report.acis_refused += 1;
                log::warn!("acis: payload of {handle:?} left as is ({reason})");
            }
            AcisConformity::Foreign | AcisConformity::AlreadyConforming => {}
        }
    }

    report.histories_removed = drop_self_owned_histories(document);
    report
}

/// Removes every solid history whose root owns the solid it hangs from, with
/// everything the root owns. Returns how many histories went. One pass over
/// the objects per ownership level — never one per solid: acadrust's
/// `delete_solid_history` walks every object of the drawing for each call.
fn drop_self_owned_histories(document: &mut CadDocument) -> usize {
    let self_owned: Vec<(Handle, Handle)> = document
        .entities()
        .filter_map(|entity| {
            let owner = entity.common().handle;
            let root = history_of(entity)?;
            match document.objects.get(&root)? {
                ObjectType::DynamicBlock(object) => match &object.data {
                    DynamicBlockData::SolidHistory(history) if history.owner == owner => {
                        Some((owner, root))
                    }
                    _ => None,
                },
                _ => None,
            }
        })
        .collect();
    if self_owned.is_empty() {
        return 0;
    }
    let mut removed: HashSet<Handle> = HashSet::with_capacity(self_owned.len() * 2);
    for (owner, root) in &self_owned {
        if let Some(entity) = document.get_entity_mut(*owner) {
            clear_history(entity);
        }
        removed.insert(*root);
    }
    // The roots, then everything they own. acadrust hangs the history nodes
    // directly under the root; the loop also follows deeper chains.
    let mut frontier: HashSet<Handle> = removed.clone();
    while !frontier.is_empty() {
        let owned: HashSet<Handle> = document
            .objects
            .iter()
            .filter_map(|(handle, object)| match object {
                ObjectType::DynamicBlock(object)
                    if frontier.contains(&object.owner) && !removed.contains(handle) =>
                {
                    Some(*handle)
                }
                _ => None,
            })
            .collect();
        removed.extend(owned.iter().copied());
        frontier = owned;
    }
    document.objects.retain(|handle, _| !removed.contains(handle));
    self_owned.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cadkernel::brep::{make, Body};

    /// The document the kernel wrote before this module existed: exactly what
    /// `acis_export::solid_to_sat` produced on 2026-09-27.
    fn legacy_document(body: &Body) -> SatDocument {
        let mut document = SatDocument::new();
        cadkernel::acis::append(body, &mut document).expect("append");
        SatDocument::parse(&document.to_sat_string()).expect("parse")
    }

    fn primitives() -> Vec<(&'static str, Body)> {
        vec![
            ("cuboid", make::cuboid([0.0; 3], [10.0, 20.0, 30.0]).unwrap()),
            ("offset cuboid", make::cuboid([100.0, 50.0, 7.0], [30.0, 15.0, 12.0]).unwrap()),
            ("cylinder", make::cylinder([0.0; 3], 5.0, 30.0).unwrap()),
            ("cone", make::cone([0.0; 3], 5.0, 30.0).unwrap()),
            ("sphere", make::sphere([1.0, 2.0, 3.0], 5.0).unwrap()),
            ("torus", make::torus([0.0; 3], 10.0, 2.0).unwrap()),
            ("wedge", make::wedge([0.0; 3], 10.0, 20.0, 30.0).unwrap()),
            ("pyramid", make::pyramid([0.0; 3], 5.0, 30.0, 4).unwrap()),
        ]
    }

    fn volume(body: &Body) -> f64 {
        cadkernel::brep::mesh_body(body, 0.05, 1e-6)
            .mass_properties()
            .map(|(volume, _)| volume)
            .expect("closed mesh")
    }

    fn only_body(document: &SatDocument) -> Body {
        let (mut bodies, loss) = cadkernel::acis::lift(document);
        assert!(loss.is_empty(), "{loss:?}");
        assert_eq!(bodies.len(), 1);
        bodies.remove(0)
    }

    fn last_words(record: &SatRecord, count: usize) -> Vec<String> {
        record.tokens[record.tokens.len() - count..]
            .iter()
            .map(|token| {
                token
                    .as_ident()
                    .or_else(|| token.as_string())
                    .unwrap_or("?")
                    .to_string()
            })
            .collect()
    }

    #[test]
    fn the_legacy_kernel_document_has_every_defect_odas_reader_trips_on() {
        // The baseline this module corrects, pinned so that a kernel which
        // starts writing the full layout by itself is noticed.
        let document = legacy_document(&make::cuboid([0.0; 3], [10.0, 20.0, 30.0]).unwrap());
        assert_eq!(document.header.num_bodies, 0);
        assert_eq!(document.header.spatial_resolution, KERNEL_UNITS);
        assert_ne!(document.records[0].entity_type, "body");
        let edge = document.records.iter().find(|r| r.entity_type == "edge").unwrap();
        assert!(is_sense(edge.tokens.last().unwrap()));
        let plane = document
            .records
            .iter()
            .find(|r| r.entity_type == "plane-surface")
            .unwrap();
        assert_eq!(shape(&plane.tokens), [Run::Pointer, Run::Numbers(9)]);
    }

    #[test]
    fn a_conformed_document_has_the_acis_700_layout() {
        for (name, body) in primitives() {
            let mut document = legacy_document(&body);
            assert!(conform_kernel_sat(&mut document), "{name}");
            let header = &document.header;
            assert_eq!(header.num_bodies, 1, "{name}");
            assert_eq!(header.spatial_resolution, ACIS_UNITS, "{name}");
            assert_eq!(header.product_version, ACIS_700_PRODUCT_VERSION, "{name}");
            assert_eq!(document.records[0].entity_type, "body", "{name}");
            for record in &document.records {
                match record.entity_type.as_str() {
                    "edge" => assert_eq!(last_words(record, 1), ["unknown"], "{name}"),
                    "straight-curve" => {
                        assert_eq!(last_words(record, 2), ["I", "I"], "{name}");
                        let values = numbers(&record.tokens);
                        let length = (values[3] * values[3]
                            + values[4] * values[4]
                            + values[5] * values[5])
                            .sqrt();
                        assert!((length - 1.0).abs() < 1e-12, "{name}: |d| = {length}");
                    }
                    "ellipse-curve" => assert_eq!(last_words(record, 2), ["I", "I"], "{name}"),
                    "plane-surface" | "sphere-surface" | "torus-surface" => assert_eq!(
                        last_words(record, 5),
                        ["forward_v", "I", "I", "I", "I"],
                        "{name}"
                    ),
                    "cone-surface" => assert_eq!(
                        last_words(record, 5),
                        ["forward", "I", "I", "I", "I"],
                        "{name}"
                    ),
                    _ => {}
                }
            }
            // Every pointer still lands on a record of the kind it named.
            for record in &document.records {
                for token in &record.tokens {
                    if let Some(index) = token.as_pointer().and_then(|p| p.index()) {
                        assert!(index < document.records.len(), "{name}");
                    }
                }
            }
        }
    }

    #[test]
    fn every_solid_the_engine_creates_comes_out_conformed() {
        // `solid_to_sat` is the funnel of every 3D command (BOX, CYLINDER,
        // EXTRUDE, the booleans...). It must hand back the conformed layout,
        // and still accept every primitive: a conformity its own validation
        // rejected would turn each command into a silent Cancel.
        for (name, body) in primitives() {
            let document = crate::scene::convert::acis_export::solid_to_sat(&body)
                .unwrap_or_else(|| panic!("{name}: solid_to_sat refused the body"));
            assert_eq!(document.header.num_bodies, 1, "{name}");
            assert_eq!(document.records[0].entity_type, "body", "{name}");
            let mut again = document.clone();
            assert!(!conform_kernel_sat(&mut again), "{name}: left unconformed");
        }
    }

    #[test]
    fn conformity_keeps_every_solid_s_geometry_through_text_and_binary() {
        for (name, body) in primitives() {
            let original_volume = volume(&body);
            let original_bounds = cadkernel::brep::body_bounds(&body).unwrap();
            let mut document = legacy_document(&body);
            assert!(conform_kernel_sat(&mut document), "{name}");
            let text = SatDocument::parse(&document.to_sat_string()).expect("SAT");
            let binary = SabReader::read(&SabWriter::write(&document)).expect("SAB");
            for (form, round_trip) in [("SAT", text), ("SAB", binary)] {
                let lifted_body = only_body(&round_trip);
                assert!(lifted_body.validate().is_empty(), "{name} {form}");
                let lifted_volume = volume(&lifted_body);
                assert!(
                    (lifted_volume - original_volume).abs() <= 1e-6 * original_volume.abs(),
                    "{name} {form}: {lifted_volume} vs {original_volume}"
                );
                let bounds = cadkernel::brep::body_bounds(&lifted_body).unwrap();
                for axis in 0..3 {
                    let low = (bounds.min[axis] - original_bounds.min[axis]).abs();
                    let high = (bounds.max[axis] - original_bounds.max[axis]).abs();
                    assert!(low < 1e-9 && high < 1e-9, "{name} {form} axis {axis}");
                }
            }
        }
    }

    #[test]
    fn straight_edges_measure_their_length_along_a_unit_direction() {
        let mut document = legacy_document(&make::cuboid([0.0; 3], [10.0, 20.0, 30.0]).unwrap());
        conform_kernel_sat(&mut document);
        let mut spans: Vec<f64> = Vec::new();
        for record in document.records.iter().filter(|r| r.entity_type == "edge") {
            let start = record.token_float(2).unwrap();
            let end = record.token_float(4).unwrap();
            spans.push((end - start).abs());
        }
        spans.sort_by(f64::total_cmp);
        assert_eq!(spans.len(), 12);
        assert_eq!(&spans[..4], &[10.0; 4]);
        assert_eq!(&spans[4..8], &[20.0; 4]);
        assert_eq!(&spans[8..], &[30.0; 4]);
    }

    #[test]
    fn conformity_is_idempotent() {
        for (name, body) in primitives() {
            let mut document = legacy_document(&body);
            assert!(conform_kernel_sat(&mut document), "{name}");
            let once = document.to_sat_string();
            let mut again = SatDocument::parse(&once).unwrap();
            assert!(!conform_kernel_sat(&mut again), "{name}");
            assert_eq!(again.to_sat_string(), once, "{name}");
            let binary = SabWriter::write(&document);
            let mut reread = SabReader::read(&binary).unwrap();
            assert!(!conform_kernel_sat(&mut reread), "{name} (SAB)");
            assert_eq!(SabWriter::write(&reread), binary, "{name} (SAB)");
        }
    }

    #[test]
    fn acis_from_another_writer_is_left_alone() {
        // The AutoCAD/ASM region shipped in acadrust's examples, and a kernel
        // document relabelled as ezdxf's: neither is ours to rewrite.
        let region = concat!(
            "700 0 1 0\n",
            "9 Reference 20 ASM 232.6.0.65535 NT 24 Wed Sep  9 17:32:36 2026\n",
            "1 9.999999999999999547e-07 1.000000000000000036e-10\n",
            "body $-1 -1 $-1 $1 $-1 $-1 #\n",
            "lump $-1 -1 $-1 $-1 $2 $0 #\n",
            "shell $-1 -1 $-1 $-1 $-1 $3 $-1 $1 #\n",
            "face $-1 -1 $-1 $-1 $4 $2 $-1 $5 forward double out #\n",
            "loop $-1 -1 $-1 $-1 $6 $3 #\n",
            "plane-surface $-1 -1 $-1 0 0 0 0 0 1 1 0 0 forward_v I I I I #\n",
            "coedge $-1 -1 $-1 $6 $6 $-1 $7 forward $4 $-1 #\n",
            "edge $-1 -1 $-1 $8 0 $8 6.2831853071795862 $6 $9 forward @7 unknown #\n",
            "vertex $-1 -1 $-1 $7 $10 #\n",
            "ellipse-curve $-1 -1 $-1 0 0 0 0 0 1 5 0 0 1 I I #\n",
            "point $-1 -1 $-1 5 0 0 #\n",
            "End-of-ACIS-data\n",
        );
        let mut document = SatDocument::parse(region).unwrap();
        let before = document.to_sat_string();
        assert!(!conform_kernel_sat(&mut document));
        assert_eq!(document.to_sat_string(), before);

        let mut relabelled = legacy_document(&make::cuboid([0.0; 3], [1.0; 3]).unwrap());
        relabelled.header.product_id = "ezdxf v1.4.4 ACIS Builder".to_string();
        let before = relabelled.to_sat_string();
        assert!(!conform_kernel_sat(&mut relabelled));
        assert_eq!(relabelled.to_sat_string(), before);

        let mut acis = AcisData::from_sat(region);
        let payload = acis.sat_data.clone();
        assert_eq!(conform_acis_data(&mut acis), AcisConformity::Foreign);
        assert_eq!(acis.sat_data, payload);
    }

    #[test]
    fn a_legacy_payload_is_repaired_in_place_text_and_binary() {
        for (name, body) in primitives() {
            let legacy = legacy_document(&body);

            let mut text = AcisData::from_sat(&legacy.to_sat_string());
            assert_eq!(conform_acis_data(&mut text), AcisConformity::Conformed, "{name}");
            assert_eq!(conform_acis_data(&mut text), AcisConformity::AlreadyConforming, "{name}");
            let repaired = SatDocument::parse(&text.sat_data).unwrap();
            assert_eq!(repaired.header.num_bodies, 1, "{name}");
            assert_eq!(repaired.records[0].entity_type, "body", "{name}");

            // What an earlier engine left in every DWG: the legacy document,
            // encoded to SAB as it was, read back as a binary payload.
            let mut binary = AcisData::from_sab(SabWriter::write(&legacy));
            assert_eq!(conform_acis_data(&mut binary), AcisConformity::Conformed, "{name}");
            assert_eq!(conform_acis_data(&mut binary), AcisConformity::AlreadyConforming, "{name}");
            let repaired = SabReader::read(&binary.sab_data).unwrap();
            assert_eq!(repaired.header.num_bodies, 1, "{name}");
            assert_eq!(repaired.header.spatial_resolution, ACIS_UNITS, "{name}");
            let original_volume = volume(&body);
            let lifted_volume = volume(&only_body(&repaired));
            assert!(
                (lifted_volume - original_volume).abs() <= 1e-6 * original_volume.abs(),
                "{name}: {lifted_volume} vs {original_volume}"
            );
        }
    }

    #[test]
    fn the_quick_header_checks_read_what_the_writers_write() {
        let legacy = legacy_document(&make::cuboid([0.0; 3], [1.0; 3]).unwrap());
        let text = AcisData::from_sat(&legacy.to_sat_string());
        assert_eq!(sat_text_header(&text.sat_data), Some((700, 0, KERNEL_PRODUCT)));
        let sab = SabWriter::write(&legacy);
        assert_eq!(sab_header(&sab), Some((700, 0, KERNEL_PRODUCT)));
        assert!(sab_header(b"ASM BinaryFile4 not ours").is_none());
        assert!(sat_text_header("garbage").is_none());
    }

    fn document_with_legacy_solid() -> (CadDocument, Handle) {
        let mut document = CadDocument::new();
        let legacy = legacy_document(&make::cuboid([0.0; 3], [10.0, 20.0, 30.0]).unwrap());
        let mut solid = acadrust::entities::Solid3D::new();
        solid.acis_data = AcisData::from_sab(SabWriter::write(&legacy));
        let handle = document
            .add_entity(EntityType::Solid3D(solid))
            .expect("add solid");
        (document, handle)
    }

    fn with_engine_history(
        document: &mut CadDocument,
        handle: Handle,
    ) -> acadrust::SolidHistoryGraph {
        document
            .create_solid_history(
                handle,
                crate::scene::model::solid_history::brep_op(
                    &make::cuboid([0.0; 3], [10.0, 20.0, 30.0]).unwrap(),
                ),
            )
            .expect("history")
    }

    #[test]
    fn saving_conforms_legacy_solids_and_drops_self_owned_histories() {
        let (mut document, handle) = document_with_legacy_solid();
        let graph = with_engine_history(&mut document, handle);
        assert!(document.objects.contains_key(&graph.root));

        let report = prepare_for_interchange(&mut document);
        assert_eq!(report.acis_conformed, 1);
        assert_eq!(report.acis_refused, 0);
        assert_eq!(report.histories_removed, 1);
        assert!(!document.objects.contains_key(&graph.root));
        for node in &graph.nodes {
            assert!(!document.objects.contains_key(node));
        }
        let Some(EntityType::Solid3D(solid)) = document.get_entity(handle) else {
            panic!("solid gone");
        };
        assert!(solid.history_handle.is_none());
        let (version, bodies, product) = sab_header(&solid.acis_data.sab_data).unwrap();
        assert_eq!((version, bodies, product), (700, 1, KERNEL_PRODUCT));

        // A second save finds nothing left to do.
        assert_eq!(prepare_for_interchange(&mut document), InterchangeReport::default());
    }

    #[test]
    fn a_history_that_owns_something_else_is_kept() {
        let (mut document, handle) = document_with_legacy_solid();
        let graph = with_engine_history(&mut document, handle);
        // AutoCAD's root owns its evaluation graph, not the solid.
        if let Some(ObjectType::DynamicBlock(object)) = document.objects.get_mut(&graph.root) {
            if let DynamicBlockData::SolidHistory(history) = &mut object.data {
                history.owner = graph.nodes[0];
            }
        }
        let report = prepare_for_interchange(&mut document);
        assert_eq!(report.histories_removed, 0);
        assert!(document.objects.contains_key(&graph.root));
        let Some(EntityType::Solid3D(solid)) = document.get_entity(handle) else {
            panic!("solid gone");
        };
        assert_eq!(solid.history_handle, Some(graph.root));
    }

    #[test]
    fn only_the_snapshot_loses_its_histories() {
        // `save_owned_as_version_inner` hands the writer a CLONE; the live
        // document keeps its history for the engine's own features.
        let (mut live, handle) = document_with_legacy_solid();
        let graph = with_engine_history(&mut live, handle);
        let mut snapshot = live.clone();
        prepare_for_interchange(&mut snapshot);
        assert!(live.objects.contains_key(&graph.root));
        assert!(live.solid_history_operations(handle).is_some());
        assert!(!snapshot.objects.contains_key(&graph.root));
    }

    #[test]
    fn a_saved_dwg_carries_conformed_acis_and_reopens_with_the_same_solid() {
        let (mut document, handle) = document_with_legacy_solid();
        prepare_for_interchange(&mut document);
        let mut bytes = std::io::Cursor::new(Vec::new());
        acadrust::DwgWriter::write_to_writer(&mut bytes, &document).expect("write DWG");
        let reopened = acadrust::DwgReader::from_stream(std::io::Cursor::new(bytes.into_inner()))
            .read()
            .expect("read DWG");
        let solid = reopened
            .entities()
            .find_map(|entity| match entity {
                EntityType::Solid3D(solid) => Some(solid),
                _ => None,
            })
            .expect("the solid survives");
        let document = solid.acis_data.parse().expect("ACIS");
        assert_eq!(document.header.num_bodies, 1);
        assert_eq!(document.records[0].entity_type, "body");
        let body = only_body(&document);
        let expected = volume(&make::cuboid([0.0; 3], [10.0, 20.0, 30.0]).unwrap());
        assert!((volume(&body) - expected).abs() < 1e-6 * expected);
        let _ = handle;
    }
}
