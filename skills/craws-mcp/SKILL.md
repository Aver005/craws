---
name: craws-mcp
description: >-
  Drive the craws image MCP server to annotate, compose, and transform images —
  above all, screenshots for documentation. Use this whenever you need to draw
  arrows, circles, rectangles, lines, or translucent highlights on an image; add
  a text label or caption; point at or mark up a UI element in a screenshot;
  redact/censor a region; overlay one image on another; build a multi-image
  collage or figure; or crop, resize, adjust exposure, or grayscale an image with
  craws. Trigger it for requests like "annotate this screenshot", "circle the
  login button", "add an arrow pointing to the menu", "label this step", "caption
  the figure", "make a collage of these screenshots", "mark this up for the docs",
  "blur out the API key", or any flow that pairs Playwright / browser screenshots
  with image editing — even if the user never says "craws".
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

Single-image edits (`resize`, `crop`, `exposure`, `grayscale`, the five `draw_*`
including `draw_text`) each take one `image_id` and return one. `overlay` and
`collage` take several handles and return one.

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
- **Redact a secret → `draw_rect`** with an **opaque** `fill` (`black`, or a solid
  brand color) over the region. (A blur op isn't available yet — a solid bar is the
  reliable redaction.)
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
  documentation pipeline, numbered-step callouts, before/after plates, redaction,
  batch-processing many screenshots, and resizing for a target doc width.
