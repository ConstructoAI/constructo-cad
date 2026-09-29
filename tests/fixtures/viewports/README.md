Paper-space viewports written by something other than AutoCAD, whose only
differences are their on/off state. Checked by `io::viewport_status_tests`.

The rule is the DXF reference's (and ezdxf's `VSF_*` constants): a viewport is
OFF if and only if its status flags (group 90, the same bit-coded long in a DWG)
carry `0x20000`, or, in a DXF, its status (group 68) is 0. `0x4000` locks the
view; `0x8000` is "currently always enabled" and means nothing — AutoCAD always
writes it, ezdxf leaves it clear by default.

## `status-flags-ezdxf-oda.dwg`

Built with ezdxf 1.4.4, then converted to DWG R2018 (AC1032) by ODA File
Converter 27.1, the path every drawing of the Constructo plan generators takes.
Inches, sheet 36 × 24. Each viewport (10 × 6.5 on paper, view height 78) looks
at its own labelled target in model space; the view is set by its center
(group 12), the target (group 17) is 0.

| Layout | Paper center | View center (model) | Group 90 | State |
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

Plus one overall paper-space viewport per layout (ODA writes `0x88020`). ODA
agrees with the table: read back to DXF, it gives the two OFF viewports id −1
(inactive) and numbers every other one.

## `status-flags-ezdxf.dxf`

Straight from ezdxf 1.4.4 (R2018, no ODA), layout `ETATS`, 7 viewports 10 × 6.5
looking at (1000 + 200 × i, 0), that differ only by their groups 90 and 68:

| View center x | Group 90 | Group 68 | State |
|---|---|---|---|
| 1000 | `0x04000` | 2 | on, locked |
| 1200 | `0x0C000` | 3 | on, locked |
| 1400 | `0x28000` | 4 | OFF (`0x20000`) |
| 1600 | `0x20000` | 5 | OFF (`0x20000`) |
| 1800 | `0x08000` | 0 | OFF (status 0) |
| 2000 | `0x04000` | −1 | on (off screen), locked |
| 2200 | `0x00000` | 6 | on |

Plus the layout's overall viewport (68 = 1, 90 = `0x88020`).
