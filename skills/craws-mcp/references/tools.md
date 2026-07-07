# craws MCP — full tool reference

Every tool returns a single JSON text line. Image-producing tools return
`{"image_id": "...", "width": N, "height": N}`; `export` returns
`{"path": "...", "bytes": N}`. Coordinates are pixels, origin top-left. Colors are
hex (`#RGB`, `#RRGGBB`, `#RRGGBBAA`) or a name
(`red green blue yellow orange white black gray grey cyan magenta transparent`).

## Contents
- [Session & I/O](#session--io): open_image, image_info, export
- [Transforms](#transforms): resize, crop, exposure, grayscale
- [Annotation](#annotation): draw_rect, draw_ellipse, draw_line, draw_arrow, draw_text
- [Composition](#composition): overlay, collage

---

## Session & I/O

### `open_image`
Load an image file into the session.

| param | type | required | notes |
|---|---|---|---|
| `path` | string | yes | filesystem path to a `.png` / `.jpg` / `.webp` |

Returns `{image_id, width, height}`. Format is sniffed from the bytes, not the
extension. Decoding failure → error.

### `image_info`
Report a handle's current dimensions without changing anything.

| param | type | required |
|---|---|---|
| `image_id` | string | yes |

Returns `{image_id, width, height}`. Use it to place elements relative to edges or
to confirm a handle is still valid.

### `export`
Write a handle to disk. **Format follows the file extension.**

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `path` | string | yes | | `.png` / `.jpg` / `.webp` |
| `quality` | int 1–100 | no | 90 | **jpeg only** |

Returns `{path, bytes}`. `.png`/`.webp` are lossless and keep alpha; `.jpg` is lossy
and flattens transparency over white. Unknown extension → error.

---

## Transforms

### `resize`
Resample to a new size (high-quality Lanczos3 by default, in linear light — no
dark-edge fringing).

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `width` | int | no* | | |
| `height` | int | no* | | |
| `filter` | string | no | `lanczos3` | `nearest` \| `bilinear` \| `catmull_rom` \| `lanczos3` |

\*Give **at least one** of `width`/`height`. Supplying only one **preserves aspect
ratio**; supplying both forces exact dimensions (may stretch). `nearest` is for
pixel-art / crisp UI upscales; `lanczos3` is best for photos and downscaling
screenshots to a doc width.

### `crop`
Keep a rectangular region. The rect **must lie fully inside** the image.

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `x`, `y` | int | yes | top-left of the crop |
| `width`, `height` | int | yes | must satisfy `x+width ≤ W`, `y+height ≤ H` |

Out-of-bounds → error (names the rect and the image size).

### `exposure`
Photographic exposure in **stops**: linear multiply by `2^stops`.

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `stops` | float | yes | `+1` doubles brightness, `-1` halves it |

### `grayscale`
Rec.709 luminance grayscale (computed in linear light).

| param | type | required |
|---|---|---|
| `image_id` | string | yes |

---

## Annotation

All draw ops **keep the image size** and return a new handle. Shapes are
anti-aliased and composited in linear light, so translucent colors blend correctly.
Coordinates are fractional pixels; shapes are clipped to the canvas.

### `draw_rect`
Rectangle with optional fill, optional outline, optional rounded corners.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `x`, `y` | float | yes | | top-left |
| `width`, `height` | float | yes | | |
| `corner_radius` | float | no | 0 | clamped to half the shorter side |
| `fill` | color | no† | | interior |
| `stroke` | color | no† | | outline |
| `stroke_width` | float | no | 3 | outline width, centered on the edge |

†**At least one of `fill`/`stroke` is required.** Outline-only = `stroke` only;
filled highlight = translucent `fill` only; both = filled box with a border.

### `draw_ellipse`
Ellipse inscribed in the `(x, y, width, height)` box. Equal width/height = circle.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `x`, `y` | float | yes | | top-left of the **bounding box** |
| `width`, `height` | float | yes | | box size (not radius) |
| `fill` | color | no† | | |
| `stroke` | color | no† | | |
| `stroke_width` | float | no | 3 | |

†At least one of `fill`/`stroke`.

### `draw_line`
Straight segment between two points.

| param | type | required | default |
|---|---|---|---|
| `image_id` | string | yes | |
| `x1`, `y1`, `x2`, `y2` | float | yes | |
| `color` | color | yes | |
| `thickness` | float | no | 3 |

### `draw_arrow`
Segment with a V arrowhead at the **second** point.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `x1`, `y1` | float | yes | | tail |
| `x2`, `y2` | float | yes | | **tip** — the thing you point at |
| `color` | color | yes | | |
| `thickness` | float | no | 3 | |
| `head_length` | float | no | 18 | arrowhead size |

### `draw_text`
Draw a text label. Anti-aliased glyphs composited in linear light; Unicode
(Latin + Cyrillic render with the built-in font).

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `x`, `y` | float | yes | | anchor point (not top-left — see align) |
| `text` | string | yes | | `\n` starts a new line |
| `color` | color | yes | | |
| `font_size` | float | no | 24 | pixel height |
| `font` | string | no | built-in | a family name (resolved from installed fonts) **or** a `.ttf`/`.otf` path; omitted → the embedded default font |
| `align_x` | string | no | `left` | `left` \| `center` \| `right` — horizontal anchoring at `x` |
| `align_y` | string | no | `baseline` | `top` \| `middle` \| `bottom` \| `baseline` — vertical anchoring at `y` |
| `line_height` | float | no | font's natural | baseline-to-baseline distance for multi-line text |

`(x, y)` is an anchor: `align_x` decides whether the text starts at, centers on, or
ends at `x`; `align_y:"baseline"` (default) puts `y` on the first line's baseline (so
text sits above `y`), while `top`/`middle`/`bottom` anchor the text block's box.
Unknown font names fall back to the built-in font; a bad `align_*` value errors.

---

## Composition

These take **multiple** handles. They are not part of the single-image chain — they
combine images you've already opened/edited.

### `overlay`
Composite `top` onto `base` at an offset. Result keeps the **base's** size.

| param | type | required | default | notes |
|---|---|---|---|---|
| `base_id` | string | yes | | the image drawn onto |
| `top_id` | string | yes | | the image pasted on top |
| `x`, `y` | int | no | 0 | top-left of `top` within `base`; may be negative to crop |
| `opacity` | float | no | 1.0 | 0..1 |

Source-over alpha compositing. To place a detail in the bottom-right corner, compute
`x = base.width - top.width - margin`, `y = base.height - top.height - margin` (get
sizes via `image_info`).

### `collage`
Arrange several handles into a justified-rows layout (each full row scaled to fill
the width; aspect ratios preserved).

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_ids` | string[] | yes | | reading order = array order; must be non-empty |
| `target_width` | int | no | 1600 | canvas width |
| `row_height` | int | no | 320 | nominal row height before justification |
| `gap` | int | no | 12 | spacing between cells and around the edges |
| `background` | color | no | white | canvas fill (use a color, or `transparent` for png) |

Returns a new handle sized `target_width × (computed height)`. More images per row
→ shorter rows; a larger `row_height` → bigger cells and a taller canvas.
