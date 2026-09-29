//! Paper-space block records the way DWG readers expect them.
//!
//! A drawing has one block record per paper layout. The DWG format singles
//! one of them out as `*Paper_Space` — the header and the block table point at
//! it, and it belongs to the layout that is current in paper space. The others
//! are ordinary blocks (`*Paper_Space0`, `*Paper_Space1`, …): their entities
//! are written as OWNED (entity mode 0, with their owner handle), and only the
//! entities of `*Paper_Space` are written in PAPER mode (entity mode 1).
//!
//! The DWG writer (cadcodec) decides the entity mode from the block NAME and
//! gives mode 1 to every `*Paper_Space<N>` block. Measured on 2026-09-28 with
//! ODA File Converter 27.1, which reads a DWG without auditing it the way a
//! plain open does: in a seven-layout drawing written by the engine, every
//! viewport of the six layouts that were not `*Paper_Space` came back with
//! id -1 and status -1 — viewports that are not active — and ezdxf then drew
//! none of their views. The same file with ODA's audit, and the same set built
//! by ezdxf, have ids 1..n on every sheet. The writer honours an explicit
//! `EntityCommon::entity_mode`, so the copy handed to it is corrected here.

use acadrust::objects::ObjectType;
use acadrust::{CadDocument, EntityType, Handle};

/// `*Paper_Space` or `*Paper_Space<digits>`, in any case.
pub(crate) fn is_paper_space_block_name(name: &str) -> bool {
    const PREFIX: &str = "*Paper_Space";
    name.get(..PREFIX.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(PREFIX))
        && name[PREFIX.len()..].chars().all(|c| c.is_ascii_digit())
}

/// Name the paper-space block records: `primary` (or, without one, the block
/// already called `*Paper_Space`, else the first layout's) becomes
/// `*Paper_Space`, the other layouts' blocks `*Paper_Space0`,
/// `*Paper_Space1`, … in tab order without a gap, and the header points at
/// the primary block.
///
/// Two ways the names broke before: deleting the layout that owned
/// `*Paper_Space` left the header and the block table pointing at nothing, and
/// deleting any other layout left a gap, so the next `CadDocument::add_layout`
/// — which names its block `*Paper_Space<count - 1>` — met a survivor of that
/// name and failed.
pub(crate) fn normalize_paper_space_blocks(document: &mut CadDocument, primary: Option<Handle>) {
    let mut owned: Vec<(i16, Handle)> = document
        .objects
        .values()
        .filter_map(|object| match object {
            ObjectType::Layout(layout)
                if layout.name != "Model" && !layout.block_record.is_null() =>
            {
                Some((layout.tab_order, layout.block_record))
            }
            _ => None,
        })
        .collect();
    owned.sort_by_key(|(order, handle)| (*order, handle.value()));
    owned.dedup_by_key(|(_, handle)| *handle);
    let blocks: Vec<(Handle, String, Handle)> = owned
        .iter()
        .filter_map(|(_, handle)| {
            document
                .block_records
                .iter()
                .find(|block| block.handle == *handle)
                .filter(|block| is_paper_space_block_name(&block.name))
                .map(|block| (block.handle, block.name.clone(), block.block_entity_handle))
        })
        .collect();
    let Some(first) = blocks.first() else {
        return;
    };
    let primary = primary
        .filter(|wanted| blocks.iter().any(|(handle, _, _)| handle == wanted))
        .or_else(|| {
            blocks
                .iter()
                .find(|(_, name, _)| name.eq_ignore_ascii_case("*Paper_Space"))
                .map(|(handle, _, _)| *handle)
        })
        .unwrap_or(first.0);
    let mut next = 0;
    let renames: Vec<(Handle, String, String, Handle)> = blocks
        .iter()
        .map(|(handle, name, marker)| {
            let target = if *handle == primary {
                "*Paper_Space".to_string()
            } else {
                let target = format!("*Paper_Space{next}");
                next += 1;
                target
            };
            (*handle, name.clone(), target, *marker)
        })
        .filter(|(_, name, target, _)| name != target)
        .collect();
    // Through unique temporary names first, so no rename meets a name another
    // block is about to give up.
    let mut staged = Vec::new();
    for (handle, name, target, marker) in renames {
        let temporary = format!("*Paper_Space_renumber_{:X}", handle.value());
        if document.block_records.rename(&name, temporary.clone()).is_ok() {
            staged.push((temporary, target, marker));
        }
    }
    for (temporary, target, marker) in staged {
        let final_name = if document.block_records.rename(&temporary, target.clone()).is_ok() {
            target
        } else {
            temporary
        };
        // The block marker carries the name the writers emit.
        if let Some(EntityType::Block(block)) = document.get_entity_mut(marker) {
            block.name = final_name;
        }
    }
    document.header.paper_space_block_handle = primary;
}

/// The block record of the paper layout the drawing is saved on, when it is
/// saved in paper space (`$TILEMODE` 0 and the `CTAB` variable naming a paper
/// layout).
fn active_paper_block(document: &CadDocument) -> Option<Handle> {
    if document.header.show_model_space {
        return None;
    }
    let name = crate::io::saved_active_layout(document)?;
    document.objects.values().find_map(|object| match object {
        ObjectType::Layout(layout)
            if layout.name == name && name != "Model" && !layout.block_record.is_null() =>
        {
            Some(layout.block_record)
        }
        _ => None,
    })
}

/// On the copy handed to the DWG writer: the paper layout the drawing is saved
/// on owns `*Paper_Space` (the reference application opens a drawing in paper
/// space on that layout), the other paper-space blocks are numbered, and every
/// entity of those other blocks is written as owned (entity mode 0 with its
/// owner handle) while the entities of `*Paper_Space` are written in paper
/// mode (1). Model space and named blocks are left to the writer.
pub(crate) fn prepare_paper_space_for_dwg(document: &mut CadDocument) {
    let active = active_paper_block(document);
    normalize_paper_space_blocks(document, active);
    let primary = document.header.paper_space_block_handle;
    let members: Vec<(Handle, Handle, u8)> = document
        .block_records
        .iter()
        .filter(|block| is_paper_space_block_name(&block.name))
        .flat_map(|block| {
            let mode = if block.handle == primary { 1 } else { 0 };
            block
                .entity_handles
                .iter()
                .map(move |entity| (*entity, block.handle, mode))
        })
        .collect();
    for (entity, owner, mode) in members {
        let Some(slot) = document.get_entity_mut(entity) else {
            continue;
        };
        if matches!(slot, EntityType::Block(_) | EntityType::BlockEnd(_)) {
            continue;
        }
        let common = slot.common_mut();
        common.owner_handle = owner;
        common.entity_mode = Some(mode);
    }
}

// ── Viewport states in a DXF ─────────────────────────────────────────────

/// Group 90 of a VIEWPORT: "currently always enabled" and off. The lock
/// (0x4000) and the other bits come from the viewport as the writer has them.
const VP_ALWAYS: i32 = 0x8000;
const VP_OFF: i32 = 0x2_0000;

/// How one viewport must read in a DXF: its id on its sheet (group 69), its
/// status (68: the stacking order when on, 0 when off) and its flags (90).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct DxfViewportState {
    pub id: i16,
    pub status: i16,
    pub flags: i32,
}

/// The state of every viewport of every paper layout, as a DXF reader needs
/// it. The sheet (overall) viewport is id 1; the others are 2, 3, … in the
/// order they were made — their own id when they carry one, otherwise their
/// place in the layout's block. An engine drawing carries id 0 on every
/// viewport read from a DWG, which does not store the ids.
pub(crate) fn dxf_viewport_states(
    document: &CadDocument,
) -> std::collections::HashMap<Handle, DxfViewportState> {
    let mut states = std::collections::HashMap::new();
    for object in document.objects.values() {
        let ObjectType::Layout(layout) = object else {
            continue;
        };
        if layout.name == "Model" || layout.block_record.is_null() {
            continue;
        }
        let order: Vec<Handle> = document
            .block_records
            .iter()
            .find(|block| block.handle == layout.block_record)
            .map(|block| block.entity_handles.clone())
            .unwrap_or_default();
        let mut viewports: Vec<(Handle, i16, usize, bool, i32)> = document
            .entities()
            .filter_map(|entity| match entity {
                EntityType::Viewport(viewport)
                    if viewport.common.owner_handle == layout.block_record =>
                {
                    let handle = viewport.common.handle;
                    let place = order
                        .iter()
                        .position(|candidate| *candidate == handle)
                        .unwrap_or(usize::MAX);
                    Some((
                        handle,
                        viewport.id,
                        place,
                        viewport.status.is_on,
                        viewport.status.to_bits(),
                    ))
                }
                _ => None,
            })
            .collect();
        if viewports.is_empty() {
            continue;
        }
        let sheet = crate::scene::Scene::layout_sheet_viewport_handle(document, layout);
        viewports.sort_by_key(|(handle, id, place, _, _)| {
            let rank = if *handle == sheet {
                0
            } else if *id > 1 {
                1
            } else {
                2
            };
            (rank, if *id > 1 { *id } else { i16::MAX }, *place, handle.value())
        });
        for (index, (handle, _, _, on, bits)) in viewports.into_iter().enumerate() {
            let id = i16::try_from(index + 1).unwrap_or(i16::MAX);
            let mut flags = (bits | VP_ALWAYS) & !VP_OFF;
            if !on {
                flags |= VP_OFF;
            }
            states.insert(
                handle,
                DxfViewportState {
                    id,
                    status: if on { id } else { 0 },
                    flags,
                },
            );
        }
    }
    states
}

/// Give the VIEWPORT entities of an ASCII DXF their state (see
/// [`DxfViewportState`]).
///
/// The DXF writer (cadcodec) writes group 69 as the drawing holds it — 0 for
/// every viewport read from a DWG — and no group 68 at all. A reader then
/// takes every viewport for off: ezdxf, and the server's preview and PDF
/// export with it, drew every sheet empty. Measured on 2026-09-28: 36
/// viewports of a seven-sheet drawing, none active. The writer's own groups
/// are kept; 68 goes before 69, as in the reference application's files, and
/// 69 and 90 get their values. Entities without a state are copied as written.
pub(crate) fn patch_dxf_viewports(
    dxf: &[u8],
    states: &std::collections::HashMap<Handle, DxfViewportState>,
) -> Vec<u8> {
    if states.is_empty() {
        return dxf.to_vec();
    }
    let eol: &[u8] = if dxf.windows(2).any(|pair| pair == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let lines: Vec<&[u8]> = dxf.split_inclusive(|byte| *byte == b'\n').collect();
    let text = |line: &[u8]| -> String {
        String::from_utf8_lossy(line).trim().to_string()
    };
    let code_of = |line: &[u8]| text(line).parse::<i32>().ok();
    let mut out = Vec::with_capacity(dxf.len() + states.len() * 16);
    let push_pair = |out: &mut Vec<u8>, code: i32, value: &str| {
        out.extend_from_slice(format!("{code:>3}").as_bytes());
        out.extend_from_slice(eol);
        out.extend_from_slice(value.as_bytes());
        out.extend_from_slice(eol);
    };
    let mut i = 0;
    while i + 1 < lines.len() {
        let is_viewport =
            code_of(lines[i]) == Some(0) && text(lines[i + 1]) == "VIEWPORT";
        if !is_viewport {
            out.extend_from_slice(lines[i]);
            out.extend_from_slice(lines[i + 1]);
            i += 2;
            continue;
        }
        let mut end = i + 2;
        while end + 1 < lines.len() && code_of(lines[end]) != Some(0) {
            end += 2;
        }
        let handle = (i..end)
            .step_by(2)
            .find(|&at| code_of(lines[at]) == Some(5))
            .and_then(|at| u64::from_str_radix(&text(lines[at + 1]), 16).ok())
            .map(Handle::new);
        let Some(state) = handle.and_then(|handle| states.get(&handle)) else {
            for line in &lines[i..end] {
                out.extend_from_slice(line);
            }
            i = end;
            continue;
        };
        let has_id = (i..end)
            .step_by(2)
            .any(|at| code_of(lines[at]) == Some(69));
        for at in (i..end).step_by(2) {
            match code_of(lines[at]) {
                // Written below, with the id.
                Some(68) => {}
                Some(69) => {
                    push_pair(&mut out, 68, &state.status.to_string());
                    push_pair(&mut out, 69, &state.id.to_string());
                }
                Some(90) => {
                    if !has_id {
                        push_pair(&mut out, 68, &state.status.to_string());
                        push_pair(&mut out, 69, &state.id.to_string());
                    }
                    push_pair(&mut out, 90, &state.flags.to_string());
                }
                _ => {
                    out.extend_from_slice(lines[at]);
                    out.extend_from_slice(lines[at + 1]);
                }
            }
        }
        i = end;
    }
    for line in &lines[i..] {
        out.extend_from_slice(line);
    }
    out
}

/// Write `document` as an ASCII DXF, viewports with their state.
pub(crate) fn write_dxf(document: &CadDocument) -> Result<Vec<u8>, String> {
    let dxf = acadrust::DxfWriter::new(document)
        .write_to_vec()
        .map_err(|error| error.to_string())?;
    Ok(patch_dxf_viewports(&dxf, &dxf_viewport_states(document)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout_block(document: &CadDocument, name: &str) -> Handle {
        document
            .objects
            .values()
            .find_map(|object| match object {
                ObjectType::Layout(layout) if layout.name == name => Some(layout.block_record),
                _ => None,
            })
            .expect("layout")
    }

    fn block_name(document: &CadDocument, handle: Handle) -> String {
        document
            .block_records
            .iter()
            .find(|block| block.handle == handle)
            .map(|block| block.name.clone())
            .expect("block")
    }

    fn line_in(document: &mut CadDocument, layout: &str) -> Handle {
        document
            .add_entity_to_layout(
                EntityType::Line(acadrust::entities::Line::new()),
                layout,
            )
            .expect("line")
    }

    #[test]
    fn block_names_are_recognised_in_any_case() {
        for name in ["*Paper_Space", "*PAPER_SPACE3", "*paper_space12"] {
            assert!(is_paper_space_block_name(name), "{name}");
        }
        for name in ["*Model_Space", "*Paper_Space_1", "*Paper", "Paper_Space2", "*U2"] {
            assert!(!is_paper_space_block_name(name), "{name}");
        }
    }

    #[test]
    fn saved_layout_owns_paper_space_and_the_others_are_written_as_owned() {
        let mut document = CadDocument::new();
        document.add_layout("A-101").unwrap();
        document.add_layout("A-102").unwrap();
        let first = line_in(&mut document, "Layout1");
        let second = line_in(&mut document, "A-101");
        let third = line_in(&mut document, "A-102");

        // Saved in Model space: *Paper_Space stays where it is.
        document.header.show_model_space = true;
        let mut copy = document.clone();
        prepare_paper_space_for_dwg(&mut copy);
        assert_eq!(block_name(&copy, layout_block(&copy, "Layout1")), "*Paper_Space");
        assert_eq!(block_name(&copy, layout_block(&copy, "A-101")), "*Paper_Space0");
        assert_eq!(block_name(&copy, layout_block(&copy, "A-102")), "*Paper_Space1");
        assert_eq!(copy.get_entity(first).unwrap().common().entity_mode, Some(1));
        for handle in [second, third] {
            let common = copy.get_entity(handle).unwrap().common().clone();
            assert_eq!(common.entity_mode, Some(0));
            assert_eq!(common.owner_handle, block_of(&copy, handle));
        }

        // Saved on A-102: that layout owns *Paper_Space, the header follows.
        document.header.show_model_space = false;
        crate::io::set_saved_active_layout(&mut document, "A-102");
        let mut copy = document.clone();
        prepare_paper_space_for_dwg(&mut copy);
        let a102 = layout_block(&copy, "A-102");
        assert_eq!(block_name(&copy, a102), "*Paper_Space");
        assert_eq!(copy.header.paper_space_block_handle, a102);
        assert_eq!(block_name(&copy, layout_block(&copy, "Layout1")), "*Paper_Space0");
        assert_eq!(block_name(&copy, layout_block(&copy, "A-101")), "*Paper_Space1");
        assert_eq!(copy.get_entity(third).unwrap().common().entity_mode, Some(1));
        assert_eq!(copy.get_entity(first).unwrap().common().entity_mode, Some(0));
        // The live document is untouched: only the writer's copy changes.
        assert_eq!(block_name(&document, layout_block(&document, "Layout1")), "*Paper_Space");
        assert_eq!(document.get_entity(first).unwrap().common().entity_mode, None);
    }

    fn block_of(document: &CadDocument, entity: Handle) -> Handle {
        document
            .block_records
            .iter()
            .find(|block| block.entity_handles.contains(&entity))
            .map(|block| block.handle)
            .expect("owning block")
    }

    #[test]
    fn gaps_and_a_missing_paper_space_are_mended() {
        let mut document = CadDocument::new();
        document.add_layout("A-101").unwrap();
        document.add_layout("A-102").unwrap();
        // Remove the *Paper_Space block the way deleting Layout1 does.
        let removed = layout_block(&document, "Layout1");
        let name = block_name(&document, removed);
        document.block_records.remove(&name);
        document.objects.retain(|_, object| {
            !matches!(object, ObjectType::Layout(layout) if layout.name == "Layout1")
        });
        normalize_paper_space_blocks(&mut document, None);
        assert_eq!(block_name(&document, layout_block(&document, "A-101")), "*Paper_Space");
        assert_eq!(block_name(&document, layout_block(&document, "A-102")), "*Paper_Space0");
        assert_eq!(
            document.header.paper_space_block_handle,
            layout_block(&document, "A-101")
        );
        // With the numbering whole again, the next layout gets a block.
        document.add_layout("A-103").unwrap();
        assert_eq!(block_name(&document, layout_block(&document, "A-103")), "*Paper_Space1");
    }

    fn viewport_in(document: &mut CadDocument, layout: &str, x: f64, on: bool) -> Handle {
        let mut viewport = acadrust::entities::Viewport::new();
        viewport.center = acadrust::types::Vector3::new(x, 5.0, 0.0);
        viewport.width = 4.0;
        viewport.height = 3.0;
        viewport.id = 0;
        viewport.status.is_on = on;
        document
            .add_entity_to_layout(EntityType::Viewport(viewport), layout)
            .expect("viewport")
    }

    /// Ids 1..n per sheet — the sheet's own viewport first, then the others
    /// in the order they were made — status = id when on, 0 and 0x20000 when
    /// off, 0x8000 always; a written DXF carries them, and the rest of the
    /// file is untouched.
    #[test]
    fn dxf_viewports_get_their_state() {
        let mut document = CadDocument::new();
        document.add_layout("A-101").unwrap();
        // `add_layout` makes the sheet viewport (id 1) itself.
        let sheet = document
            .objects
            .values()
            .find_map(|object| match object {
                ObjectType::Layout(layout) if layout.name == "A-101" => Some(layout.viewport),
                _ => None,
            })
            .expect("A-101");
        let first = viewport_in(&mut document, "A-101", 5.0, true);
        let off = viewport_in(&mut document, "A-101", 10.0, false);
        let locked = viewport_in(&mut document, "A-101", 15.0, true);
        if let Some(EntityType::Viewport(viewport)) = document.get_entity_mut(locked) {
            viewport.status.locked = true;
        }
        let other_sheet = viewport_in(&mut document, "Layout1", 18.0, true);

        let states = dxf_viewport_states(&document);
        let state = |handle| states[&handle];
        assert_eq!(state(sheet).id, 1);
        assert_eq!(state(first).id, 2);
        assert_eq!(state(off).id, 3);
        assert_eq!(state(locked).id, 4);
        assert_eq!(state(other_sheet).id, 1, "ids count per sheet");
        for handle in [sheet, first, locked, other_sheet] {
            assert_eq!(state(handle).status, state(handle).id);
            assert_eq!(state(handle).flags & (VP_ALWAYS | VP_OFF), VP_ALWAYS);
        }
        assert_eq!(state(off).status, 0);
        assert_eq!(state(off).flags & (VP_ALWAYS | VP_OFF), VP_ALWAYS | VP_OFF);
        assert_ne!(state(locked).flags & 0x4000, 0);
        assert_eq!(state(first).flags & 0x4000, 0);

        let written = String::from_utf8(write_dxf(&document).unwrap()).unwrap();
        let plain = String::from_utf8(
            acadrust::DxfWriter::new(&document).write_to_vec().unwrap(),
        )
        .unwrap();
        // One pair more per viewport (68), the others rewritten in place.
        assert_eq!(written.lines().count(), plain.lines().count() + 2 * 5);
        let pairs: Vec<(i32, String)> = written
            .lines()
            .collect::<Vec<_>>()
            .chunks(2)
            .filter_map(|pair| Some((pair[0].trim().parse().ok()?, pair.get(1)?.trim().to_string())))
            .collect();
        let off_hex = format!("{:X}", off.value());
        let at = pairs
            .iter()
            .position(|(code, value)| *code == 5 && *value == off_hex)
            .expect("the off viewport");
        let entity: Vec<&(i32, String)> = pairs[at..]
            .iter()
            .take_while(|(code, _)| *code != 0)
            .collect();
        let group = |code: i32| {
            entity
                .iter()
                .find(|(c, _)| *c == code)
                .map(|(_, value)| value.clone())
                .unwrap_or_default()
        };
        assert_eq!(group(68), "0");
        assert_eq!(group(69), "3");
        assert_eq!(group(90).parse::<i32>().unwrap() & (VP_ALWAYS | VP_OFF), VP_ALWAYS | VP_OFF);
        let index = |code: i32| entity.iter().position(|(c, _)| *c == code).unwrap();
        assert_eq!(index(68) + 1, index(69), "68 right before 69");

        // Nothing to state: the bytes pass through.
        let bytes = plain.clone().into_bytes();
        assert_eq!(patch_dxf_viewports(&bytes, &Default::default()), bytes);
    }
}
