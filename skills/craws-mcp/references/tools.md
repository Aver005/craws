# craws MCP — full tool reference

Every tool returns a single JSON text line. Image-producing tools return
`{"image_id": "...", "width": N, "height": N}`; `export` returns
`{"path": "...", "bytes": N}`. Coordinates are pixels, origin top-left. Colors are
hex (`#RGB`, `#RRGGBB`, `#RRGGBBAA`) or a name
(`red green blue yellow orange white black gray grey cyan magenta transparent`).

## Contents
- [Session & I/O](#session--io): open_image, image_info, export
- [Transforms](#transforms): resize, crop, rotate, flip, pad, trim, exposure, grayscale, hue_rotate, invert
- [Color & tone](#color--tone): brightness_contrast, saturation, levels, curves, white_balance, gradient_map
- [Filters](#filters): blur, sharpen, vignette, redact, spotlight, beautify
- [Annotation](#annotation): draw_rect, draw_ellipse, draw_line, draw_arrow, draw_text
- [Composition & compare](#composition--compare): overlay, collage, diff
- [Meta](#meta): run_pipeline

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

### `rotate`
Rotate clockwise about the center. Multiples of 90° are **lossless** (exact pixel
permutation); other angles resample (bilinear) with transparent corners.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `degrees` | float | yes | | clockwise; e.g. 90, -90, 45 |
| `expand` | bool | no | true | grow the canvas to fit the rotated image; `false` keeps the original size and clips the corners |

Size: 90°/270° swap width/height; arbitrary angles with `expand` grow to the rotated
bounding box.

### `flip`
Mirror across an axis. Size unchanged.

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `axis` | string | yes | `horizontal` (left↔right) or `vertical` (top↔bottom) |

### `pad`
Extend the canvas with a colored margin. Output grows by the margins.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `all` | int | no | 0 | applied to any side left unset — uniform padding |
| `left`, `right`, `top`, `bottom` | int | no | `all` | per-side override |
| `color` | color | no | transparent | border fill (use png/webp to keep transparency) |

The inverse of `trim`. Output size = `W + left + right` × `H + top + bottom`.

### `trim`
Auto-crop a uniform border (whitespace, a solid background, or transparency).

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `color` | color | no | top-left pixel | background to remove; omit to auto-detect from the corner |
| `tolerance` | float | no | 0.01 | 0..1 match slack — raise for JPEG / anti-aliased edges |

Returns the cropped handle. A single-color image (nothing to keep) → error; an image
with no trimmable border comes back unchanged.

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

### `hue_rotate`
Rotate the hue of every pixel about the luma axis (luminance preserved).

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `degrees` | float | yes | 0..360; 0/360 = identity |

### `invert`
Photographic negative, computed in perceptual sRGB space (mid-gray → mid-gray).
Alpha is preserved.

| param | type | required |
|---|---|---|
| `image_id` | string | yes |

---

## Color & tone

Single-image, size-preserving. These are **color grades** — authored in perceptual
sRGB (like a photo editor), except `white_balance` (linear channel gains). Alpha kept.

### `brightness_contrast`
| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `brightness` | float | no | 0 | additive, ~ -1..1 |
| `contrast` | float | no | 0 | S-curve around mid-gray, ~ -1..1 |

### `saturation`
| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `amount` | float | yes | 1 = identity, 0 = grayscale, >1 boosts |

### `levels`
Remap tones per channel: pull in the black/white points, bend the midtones (gamma),
and set the output range. Omitted fields are identity.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `in_black` | float | no | 0 | input black point (0..1) |
| `in_white` | float | no | 1 | input white point (0..1) |
| `gamma` | float | no | 1 | >1 brightens midtones, <1 darkens |
| `out_black` | float | no | 0 | output black |
| `out_white` | float | no | 1 | output white |

### `curves`
Arbitrary tone curve from control points.

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `points` | array of `[x, y]` | yes | 0..1, sorted by x; e.g. `[[0,0],[0.25,0.18],[1,1]]` (an S-curve for punch) |

Applied per channel in sRGB; between points the curve is linear, outside it holds flat.

### `white_balance`
Warm/cool and green/magenta correction, as **linear** per-channel gains.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `temperature` | float | no | 0 | blue↔amber, ~ -1..1; positive = warmer |
| `tint` | float | no | 0 | green↔magenta, ~ -1..1 |

### `gradient_map`
Map each pixel's luminance to a color gradient — duotone, sepia, heatmap, split-tone.

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `low` | color | yes | color for the darkest tones |
| `high` | color | yes | color for the brightest tones |
| `mid` | color | no | optional midtone color (3-stop gradient) |

---

## Filters

Single-image, size-preserving (except `beautify`). Composited in linear light.

### `blur`
Whole-image Gaussian blur.

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `radius` | float | yes | blur strength ≈ Gaussian sigma in px (0 = no-op) |

For a *region only*, use `redact` with `mode:"blur"`.

### `sharpen`
Unsharp mask (`in + amount·(in − blur(in))`), in linear light.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `amount` | float | no | 1 | strength (0 = no-op) |
| `radius` | float | no | 2 | blur sigma of the mask |

### `vignette`
Radial darkening toward `color` (default black), for focus.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `amount` | float | no | 0.5 | strength 0..1 |
| `feather` | float | no | 0.5 | softness 0..1 (higher = falloff starts nearer the center) |
| `color` | color | no | black | color to darken toward |

### `redact`
Obscure a rectangular region — the secret-hiding tool. Size unchanged.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `x`, `y`, `width`, `height` | float | yes | | region to censor |
| `mode` | string | no | `pixelate` | `pixelate` \| `blur` \| `fill` |
| `block` | int | no | 12 | **pixelate**: mosaic cell size |
| `radius` | float | no | 12 | **blur**: Gaussian sigma |
| `color` | color | no | black | **fill**: bar color |

`pixelate` and `fill` destroy the value (safe redaction); `blur` only softens it —
don't blur real secrets. Only tiles overlapping the region are recomputed.

### `spotlight`
Dim everything *outside* the (optionally rounded, feathered) window to draw the eye
to it. Size unchanged.

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `x`, `y`, `width`, `height` | float | yes | | the window kept bright |
| `dim` | float | no | 0.55 | veil opacity outside, 0..1 (1 = fully dark) |
| `corner_radius` | float | no | 0 | round the window |
| `color` | color | no | black | veil color |
| `feather` | float | no | 0 | soft edge width in px |

### `beautify`
Polish a screenshot: round corners + soft drop shadow + padded background, in one
call. **Output grows by `2·padding` per dimension.**

| param | type | required | default | notes |
|---|---|---|---|---|
| `image_id` | string | yes | | |
| `padding` | int | no | 64 | background margin around the image |
| `corner_radius` | float | no | 16 | image corner rounding |
| `shadow_radius` | float | no | 24 | drop-shadow softness (blur sigma) |
| `shadow_opacity` | float | no | 0.35 | 0..1 (0 = no shadow) |
| `shadow_offset` | float | no | 12 | shadow vertical offset in px |
| `background` | color | no | transparent | frame color (use png/webp for transparent) |

Do it **last**, after annotation. Default transparent background = a rounded image
with a soft shadow on transparency (drops onto any doc backdrop).

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

## Composition & compare

These take **multiple** handles. They are not part of the single-image chain — they
combine or compare images you've already opened/edited.

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

### `diff`
Compare two images: a visualization **plus a change metric**.

| param | type | required | default | notes |
|---|---|---|---|---|
| `a_id` | string | yes | | baseline |
| `b_id` | string | yes | | image compared against the baseline |
| `view` | string | no | `heatmap` | `heatmap` \| `difference` \| `side_by_side` |
| `threshold` | float | no | 0 | per-channel change threshold 0..1 for the metric |

Returns `{image_id, width, height, fraction_changed, max_difference}` — the last two
are 0..1 (or `null` when the sizes differ). `difference`/`heatmap` **require both
images the same size**; `side_by_side` accepts any (canvas = `aw + gap + bw` wide).
`heatmap` glows changed pixels red over a dimmed base; `difference` is the raw
per-channel delta. Use `fraction_changed` for visual-regression asserts.

---

## Meta

### `run_pipeline`
Apply a whole chain of single-image ops in one call.

| param | type | required | notes |
|---|---|---|---|
| `image_id` | string | yes | |
| `pipeline` | string | yes | JSON: a bare array of op objects, or `{"version":0,"steps":[...]}` |

Ops use the pipeline format — the same `{"op":...}` objects the CLI's `craws run`
consumes (every OpSpec above: resize/crop/rotate/flip/pad/trim/exposure/grayscale/
hue_rotate/invert/blur/redact/spotlight/beautify/draw_*). Returns the final handle.
Example `pipeline`:
`[{"op":"trim"},{"op":"resize","width":1200},{"op":"beautify","padding":48}]`.
(`overlay`/`collage`/`diff` are multi-image, so they're not valid pipeline steps.)
