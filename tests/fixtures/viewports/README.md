`status-flags-ezdxf-oda.dwg` — paper-space viewports whose only difference is
their status flags (DXF group 90), written by a writer other than AutoCAD:
built with ezdxf 1.4.4, then converted to DWG R2018 (AC1032) by ODA File
Converter 27.1, the path every drawing of the Constructo plan generators takes.
Inches, sheet 36 × 24. Each viewport (10 × 6.5 on paper, view height 78) looks
at its own labelled target in model space; the view is set by its center
(group 12), the target (group 17) is 0.

| Layout | Paper center | View center (model) | Group 90 | Per the DXF reference |
|---|---|---|---|---|
| `DRAPEAUX` | 6.5, 16 | 1000, 0 | `0x04000` | on, locked |
| `DRAPEAUX` | 18, 16 | 1200, 0 | `0x0C000` | on, locked |
| `DRAPEAUX` | 29.5, 16 | 1400, 0 | `0x08000` | on |
| `DRAPEAUX` | 6.5, 6.5 | 1600, 0 | `0x00000` | on |
| `DRAPEAUX` | 18, 6.5 | 1800, 0 | `0x28000` | OFF |
| `DRAPEAUX` | 29.5, 6.5 | 2000, 0 | `0x20000` | OFF |
| `LOIN` | 6.5, 16 | 0, 0 | `0x0C000` | on, locked |
| `LOIN` | 18, 16 | 216000, 0 | `0x0C000` | on, locked |
| `LOIN` | 29.5, 16 | 0, −528000 | `0x0C000` | on, locked |
| `LOIN` | 6.5, 6.5 | 216000, −528000 | `0x0C000` | on, locked |
| `LOIN` | 18, 6.5 | 217200, −528000 | `0x08000` | on |
| `LOIN` | 29.5, 6.5 | 218400, −528000 | `0x0C000` on non-plotting layer `FENETRES-NP` | on, locked |

Plus one overall paper-space viewport per layout (ODA writes `0x88020`).
Checked by `io::viewport_status_tests::viewports_written_by_ezdxf_and_oda_load_on`.

The reference (and ezdxf's `VSF_*` constants): `0x4000` locks the view,
`0x8000` is "currently always enabled", `0x20000` turns the viewport off. ODA
agrees: read back to DXF it gives the two OFF viewports id −1 (inactive) and
numbers every other one. AutoCAD always sets `0x8000`, so a viewport lacking it
is on — what ezdxf, and anything built on it, writes by default.
