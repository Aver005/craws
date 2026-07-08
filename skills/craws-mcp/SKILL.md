---
name: craws-mcp
description: >-
  Drive the craws image MCP server to annotate, compose, transform, and compare
  images — above all, screenshots for documentation. Use this whenever you need to
  draw arrows, circles, rectangles, lines, or translucent highlights on an image;
  add a text label or caption; point at or mark up a UI element; redact/censor a
  region (pixelate, blur, or black-bar); blur, rotate, flip, crop, pad, or
  auto-trim whitespace; spotlight a region by dimming the rest; polish/"beautify" a
  shot with rounded corners, a drop shadow, and a padded background; overlay one
  image on another; build a multi-image collage; adjust exposure, hue, invert, or
  grayscale; diff two images (with a change metric) for visual regression; or run a
  whole edit pipeline in one call. Trigger it for requests like "annotate this
  screenshot", "circle the login button", "add an arrow to the menu", "label this
  step", "blur out the API key", "black-bar the email", "trim the whitespace",
  "round the corners and add a shadow", "make this look nice for the docs",
  "spotlight the toolbar", "make a collage", "compare before and after", "rotate
  this", or any flow pairing Playwright / browser screenshots with image
  editing — even if the user never says "craws".
---

# Using the craws image MCP

`craws` is a blazing-fast Rust image engine exposed as MCP tools. You open images,
apply operations, and export results — the same automation a person would do in an
editor, but driven by tool calls. Its sweet spot is **turning raw screenshots into
finished documentation figures**: boxes around buttons, arrows to menus, circled
icons, redacted secrets, and tidy multi-shot collages.

The tools appear as `open_image`, `draw_arrow`, `collage`, etc. (in some hosts
prefixed, e.g. `mcp__craws__draw_arrow`). Call them by these names.

## The one mental model that matters: immutable handles

Every image in a craws session is **immutable and addressed by a handle** like
`img-3`. You never edit an image in place. Instead, **every operation returns a
NEW handle**, and you build up an edit by **chaining the returned `image_id` into
the next call**.

Each tool replies with a compact JSON line. Read it, take `image_id`, use it next:

```
open_image {path}                              → {"image_id":"img-1","width":1440,"height":900}
draw_rect  {image_id:"img-1", ...}             → {"image_id":"img-2","width":1440,"height":900}
draw_arrow {image_id:"img-2", ...}             → {"image_id":"img-3", ...}
export     {image_id:"img-3", path:"out.png"}  → {"bytes":51234, "path":"out.png"}
```

Why it's built this way: images are content-addressed and cached, so branching
(make two variants from `img-2`) and keeping intermediates is free — but it means
**a stale handle is the #1 mistake.** If you draw three annotations, thread the id
through all three; don't keep passing `img-1`. When in doubt, `image_info` echoes a
handle's current size.

## The canonical workflow

1. **Open** every source image → get handles (`open_image`).
2. **Transform / annotate**, chaining `image_id` each step.
3. **Compose** if you have several images (`overlay`, `collage`).
4. **Export** the final handle to a file (`export`; format follows the extension).

There are **34 tools** in two families:

- **Single-image ops** — take one `image_id`, return one (chain them):
  - transform: `resize` `crop` `rotate` `flip` `pad` `trim` `exposure` `grayscale`
  - color/tone: `hue_rotate` `invert` `brightness_contrast` `saturation` `levels` `curves`
    `white_balance` `gradient_map`
  - filter: `blur` `sharpen` `vignette` `redact` `spotlight` `beautify`
  - annotation: `draw_rect` `draw_ellipse` `draw_line` `draw_arrow` `draw_text`
- **Multi-image / meta ops**:
  - `overlay` `collage` take several handles → one; `diff` compares two → a visualization **plus a
    change metric**; `run_pipeline` applies a whole JSON chain of the single-image ops in one call.

Plus I/O: `open_image`, `image_info`, `export`.

## Coordinates: pixels, top-left origin — and where to get them

All geometry is in **pixels**, origin **top-left**, x → right, y → down. Draw
coordinates accept fractional pixels. Shapes may extend past the edges; they're
clipped. Drawing never changes the image's size.

The hard part of annotating a screenshot is not drawing — it's knowing *where* the
button is. **Do not guess coordinates.** Get them from the same source that made the
screenshot:

- **Playwright / browser automation** — an element's `boundingBox()` returns
  `{x, y, width, height}` in the screenshot's own pixel space. Feed those four
  numbers straight into `draw_rect` to box the element, or aim an arrow at its
  center. **Retina caveat:** if the screenshot was captured at
  `deviceScaleFactor` / DPR > 1, multiply the bounding box by that factor —
  screenshot pixels = CSS pixels × DPR. The cleanest fix is to capture at scale 1.
- **You already cropped/resized it** — track the transform yourself (a crop shifts
  the origin; a resize scales every coordinate by `new/old`).
- **`image_info`** gives you width/height to place things relative to edges
  (e.g., a caption bar along the bottom, a badge in a corner).

This "get the box from Playwright, hand it to craws" hop is the whole trick behind
automated annotated docs. See `references/cookbook.md` for the end-to-end flow.

## Colors

Pass colors as **hex** (`#RRGGBB`, `#RGB`, or `#RRGGBBAA` for alpha) or a **name**
(`red green blue yellow orange white black gray cyan magenta transparent`).
Alpha defaults to fully opaque. Translucency composites correctly (in linear
light), so `#22c55e55` is a real 33%-opacity green wash you can lay over content.

## Annotation recipes (the core use case)

- **Point at something → `draw_arrow`.** The tip is the *second* point: the head
  lands at `(x2, y2)`. Put `(x2,y2)` just off the target's edge and `(x1,y1)`
  ~60–100px away in open space. Default `thickness` 3, `head_length` 18.
- **Box / highlight a region → `draw_rect`** with a `stroke` (outline only). Add
  `corner_radius` for a softer callout. For a filled highlight instead of an
  outline, use a translucent `fill` (e.g. `#facc5533`).
- **Circle an icon → `draw_ellipse`** with a `stroke`; equal `width`/`height`
  draws a perfect circle. Great for small round targets (avatars, status dots).
- **Redact a secret → `redact`** over the region. `mode:"pixelate"` (default) is the
  safest censor — the value is gone, and unlike blur it can't be de-blurred; tune
  `block` for cell size. `mode:"fill"` paints a solid bar (`color`, default black);
  `mode:"blur"` softens (`radius`) — looks nice but is weaker, avoid it for true
  secrets. Prefer `redact` over a `draw_rect` fill: it's purpose-built and reads as
  a redaction.
- **Underline / connector → `draw_line`.**
- **Label / caption a step → `draw_text`.** Anchor at `(x, y)`; control how the text
  sits on that point with `align_x` (`left`/`center`/`right`) and `align_y`
  (`top`/`middle`/`bottom`/`baseline`). Set `font_size` (px). `font` is optional — a
  family name resolved from installed fonts (e.g. `"Segoe UI"`, `"Arial"`) or a path
  to a `.ttf`/`.otf`; omit it for the built-in font. `\n` starts a new line
  (`line_height` tunes spacing). Text is Unicode — Latin and Cyrillic render out of
  the box. To number a badge, center the text (`align_x:"center"`, `align_y:"middle"`)
  on a filled circle's center so the digit sits inside it.

At least one of `fill`/`stroke` is required for rect/ellipse — a shape with neither
is rejected. Keep stroke widths readable at the doc's final display size: 4–6px on a
full-resolution screenshot usually survives downscaling; hairlines vanish.

**Order matters** — draw back-to-front. Lay a translucent highlight first, then the
outline/arrow on top, so the callout reads clearly.

## Composition recipes

- **`overlay`** pastes `top_id` onto `base_id` at `(x, y)` with `opacity`; the
  result keeps the base's size. Use it to drop a zoomed detail into a corner, stack
  a badge/watermark, or build before/after plates. Negative `x`/`y` crop the top.
- **`collage`** arranges several handles into a **smart justified-rows layout**
  (each full row is scaled to fill `target_width`, aspect ratios preserved — the
  Flickr/Google-Photos look). Tune `target_width`, `row_height`, `gap`, and
  `background` (a color). Order of `image_ids` is the reading order. Ideal for a
  "here are the three screens" figure without hand-placing anything.

## Transform, filter & polish recipes

- **Trim whitespace → `trim`.** Auto-crops a uniform border (white margins, a solid
  background, or transparency). By default it reads the background from the top-left
  pixel; pass `color` to force one, and bump `tolerance` (0..1) for JPEG/anti-aliased
  edges. The inverse of `pad` (which *adds* a margin, any `color`, `all` for uniform).
- **Rotate / mirror → `rotate` / `flip`.** `rotate` is clockwise `degrees` (90° steps
  are lossless; `expand:true`, the default, grows the canvas so nothing clips). `flip`
  takes `axis:"horizontal"|"vertical"`.
- **Blur → `blur`** (`radius` ≈ strength). Whole-image; for a *region* use `redact`
  with `mode:"blur"`.
- **Spotlight a region → `spotlight`.** Dims everything *outside* the rectangle so the
  eye lands on it — great for "look here" in a busy UI. `dim` is the veil strength
  (0..1, default 0.55); `corner_radius` and `feather` soften the window. Opposite
  intent to a highlight box: darken the surroundings instead of outlining the target.
- **Polish a screenshot → `beautify`.** One call adds rounded corners + a soft drop
  shadow + a padded background (`padding`, `corner_radius`, `shadow_*`, `background`).
  The output **grows by `2·padding`**. Default `background` is transparent (export png)
  — set a color for a solid card. This is the "make my raw screenshot look designed"
  button; do it **last**, after any annotation.
- **Recolor → `hue_rotate`** (shift hue by `degrees`, luminance preserved) or
  **`invert`** (photographic negative — handy for a quick dark-mode mock).
- **Color-grade → `brightness_contrast` / `saturation` / `levels` / `curves` /
  `white_balance` / `gradient_map`.** Photo-editor tone tools (in sRGB): add punch with
  a `curves` S-curve (`points:[[0,0],[0.25,0.18],[0.75,0.82],[1,1]]`), warm a shot with
  `white_balance`, or apply a duotone/sepia with `gradient_map` (low→high colors).
  `sharpen` (unsharp) crisps a downscaled screenshot; `vignette` darkens the edges to
  pull the eye to the center.

## Compare & automate

- **Diff two images → `diff`.** Returns a visualization handle **and a change metric**:
  `fraction_changed` (0..1) and `max_difference` (0..1). `view:"heatmap"` (default)
  glows changed pixels red over a dimmed base; `"difference"` is the raw per-channel
  delta; `"side_by_side"` lays them out with a gap. `difference`/`heatmap` need the two
  images to be the **same size**; `side_by_side` accepts any. Use the metric for visual
  regression ("did this UI change?" → assert `fraction_changed` is near 0).
- **Run a whole chain → `run_pipeline`.** Pass `pipeline` as a JSON array of op objects
  (or `{"steps":[...]}`) to apply many single-image ops in one call — e.g.
  `[{"op":"trim"},{"op":"resize","width":1200},{"op":"beautify","padding":48}]`. Same
  op names/params as the pipeline format (see `references/tools.md`). Great for
  normalizing a batch of shots identically.

## Export

`export` picks the format from the file extension:

- **`.png` / `.webp`** — lossless, **preserve transparency**. Default choice for
  annotated screenshots and anything with translucent overlays.
- **`.jpg`** — lossy (`quality` 1–100, default 90) and **flattens any transparency
  over white**. Fine when the base is a fully opaque screenshot; avoid if the image
  has meaningful alpha you want to keep.

## Pitfalls checklist

- **Stale handle** — always chain the *returned* `image_id`. This is the most common
  error by far.
- **Arrow direction** — head is at `(x2,y2)`; that's the thing you're pointing *at*.
- **Empty shape** — rect/ellipse need a `fill` or a `stroke` (or both).
- **Text placement** — `(x,y)` is an anchor, not the top-left; a caption that lands
  in the wrong spot usually means the wrong `align_x`/`align_y`. `align_y:"baseline"`
  (the default) puts `y` on the first line's baseline, so glyphs sit *above* `y`.
- **Crop bounds** — the crop rect must lie fully inside the image, or it errors.
- **Aspect on resize** — give `width` *or* `height` to scale proportionally; give
  both to force exact (possibly stretched) dimensions.
- **DPR on retina screenshots** — scale Playwright boxes by the device pixel ratio.
- **jpeg + transparency** — transparency goes white; use png/webp to keep it.
- **Redaction strength** — `pixelate`/`fill` destroy the value; `blur` only softens it.
  Never `blur` a real secret; pixelate or bar it.
- **`diff` size rule** — `difference`/`heatmap` require both images the same size (else
  a clean error); use `side_by_side` for mismatched sizes.
- **`beautify` / `pad` grow the image** — output is bigger (beautify: +`2·padding`).
  Do `beautify` *last*, and re-read the size (`image_info`) before placing anything else.
- **`trim` needs a border** — a single-color image gives a "nothing to trim" error; and
  it detects the background from the **top-left pixel** unless you pass `color`.

Tool errors (bad handle, out-of-bounds crop, unknown color, unsupported extension)
come back as normal tool errors with a readable message — they don't kill the
session, so read the message, fix the argument, and retry.

## Worked example — annotate a login screen for a how-to step

```
open_image  {path:"login.png"}                 → img-1  (1440×900)
# red rounded box around the Sign-in button (box from Playwright boundingBox)
draw_rect   {image_id:"img-1", x:560, y:520, width:320, height:56,
             corner_radius:10, stroke:"#e11d48", stroke_width:5}   → img-2
# blue arrow pointing at the email field
draw_arrow  {image_id:"img-2", x1:360, y1:300, x2:565, y2:360,
             color:"#2563eb", thickness:6}                          → img-3
# translucent yellow highlight over the "remember me" row
draw_rect   {image_id:"img-3", x:560, y:600, width:320, height:40,
             fill:"#facc5533"}                                      → img-4
export      {image_id:"img-4", path:"step-1-signin.png"}            → done
```

## Deeper reference

- `references/tools.md` — every tool, every parameter, defaults, return shape, and
  error conditions. Read it when you need an exact signature.
- `references/cookbook.md` — extended recipes: the full Playwright → craws → Outline
  documentation pipeline, numbered-step callouts, before/after plates, redaction
  (pixelate/bar), **beautify** a hero screenshot, **diff** for visual regression,
  **run_pipeline** to normalize a batch, and resizing for a target doc width.
