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
}
