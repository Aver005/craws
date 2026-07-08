# craws MCP — cookbook

Concrete, end-to-end recipes. Coordinates are pixels (top-left origin). Every step
threads the returned `image_id` into the next call.

## Contents
- [Playwright → craws → Outline: annotated docs](#playwright--craws--outline-annotated-docs)
- [Getting element coordinates right (DPR / retina)](#getting-element-coordinates-right-dpr--retina)
- [Numbered-step callouts](#numbered-step-callouts)
- [Before / after plate](#before--after-plate)
- [Redacting secrets](#redacting-secrets)
- [Beautify a hero screenshot](#beautify-a-hero-screenshot)
- [Visual regression with diff](#visual-regression-with-diff)
- [Picture-in-picture zoom detail](#picture-in-picture-zoom-detail)
- [Resizing screenshots for a doc width](#resizing-screenshots-for-a-doc-width)
- [Batch-processing many screenshots](#batch-processing-many-screenshots)

---

## Playwright → craws → Outline: annotated docs

The flagship flow: automate a browser, screenshot a page, mark it up, and publish.
craws is the middle hop that turns a raw capture into a finished figure.

1. **Capture with Playwright.** Navigate, then take a screenshot to a file. Capture
   at `deviceScaleFactor: 1` so screenshot pixels equal CSS pixels (see the DPR note
   below). For an element figure, screenshot the element or the full page.
2. **Read the coordinates you need.** For each element you'll point at or box, grab
   `await locator.boundingBox()` → `{x, y, width, height}`. These are already in the
   screenshot's pixel space (at DPR 1). Keep them next to your annotation plan.
3. **Annotate with craws.** `open_image` the screenshot, then chain draws:
   - box an element: `draw_rect {x, y, width, height, stroke:"#e11d48", stroke_width:5, corner_radius:8}`
     using the boundingBox verbatim.
   - point at an element: aim the arrow tip at the box's edge midpoint, e.g. tip
     `(x, y + height/2)` (left edge) and tail ~80px further left.
   - circle a small icon: `draw_ellipse {x, y, width, height, stroke:"#f59e0b", stroke_width:5}`.
4. **Export** to `.png` (keeps crisp edges and any translucent highlight).
5. **Publish to Outline.** Upload the PNG as an attachment and embed it in the doc
   via the Outline MCP (create/append a document, insert the image). One screenshot
   per step, each annotated to match its instruction.

Worked snippet (values are Playwright boundingBoxes you fetched):

```
open_image {path:"capture.png"}                                  → img-1
draw_rect  {image_id:"img-1", x:612, y:410, width:196, height:48,
            stroke:"#e11d48", stroke_width:5, corner_radius:8}   → img-2   # the "Save" button
draw_arrow {image_id:"img-2", x1:470, y1:300, x2:612, y2:434,
            color:"#2563eb", thickness:6}                        → img-3   # arrow to it
export     {image_id:"img-3", path:"step-3-save.png"}            → done
# → upload step-3-save.png to Outline and embed under the "Save your work" step
```

## Getting element coordinates right (DPR / retina)

A screenshot taken at `deviceScaleFactor: N` has **N× as many pixels** as the CSS
layout. `boundingBox()` is in CSS pixels, so on a 2× capture an element at CSS
`x:100` sits at screenshot pixel `200`.

- **Simplest:** capture at `deviceScaleFactor: 1`; then boundingBox maps 1:1.
- **If you must use a hi-DPI capture:** multiply every coordinate and size by the
  DPR before handing it to craws — `craws_x = box.x * dpr`, and likewise for
  `y/width/height`. A mismatch here is why arrows land in the wrong place.
- **If you cropped or resized first:** apply that transform to the coordinates too. A
  `crop {x:cx, y:cy}` subtracts `(cx, cy)` from every later coordinate; a `resize`
  by factor `f` multiplies them by `f`.

## Numbered-step callouts

Put a numbered badge next to each highlighted element: a filled circle with the step
number centered inside it. Center the digit on the circle's center
(`align_x:"center"`, `align_y:"middle"`) so it sits dead-center:

```
# highlight the target, drop a badge, number it — (cx, cy) is the badge center
draw_rect    {image_id:img, x:.., y:.., width:.., height:.., stroke:"#2563eb", stroke_width:4} → a
draw_ellipse {image_id:a, x:cx-16, y:cy-16, width:32, height:32, fill:"#2563eb"}               → b
draw_text    {image_id:b, x:cx, y:cy, text:"1", color:"white", font_size:20,
              align_x:"center", align_y:"middle"}                                              → c
```

## Before / after plate

Two states side by side, same size, on one canvas — use `collage` with two handles
and a two-up target width, or `overlay` onto a padded background for precise control:

```
open_image {path:"before.png"} → a
open_image {path:"after.png"}  → b
collage {image_ids:[a, b], target_width:1600, row_height:450, gap:20, background:"#0f172a"} → plate
export  {image_id:plate, path:"before-after.png"}
```

Add an arrow or label on either half *before* collaging (annotate `a`/`b`, then pass
the annotated handles to `collage`).

## Redacting secrets

Use `redact` — it's purpose-built. **`pixelate`** (default) or **`fill`** truly destroy
the value; `blur` only softens it, so never blur a real secret.

```
# mosaic-censor an API key (the value is gone, and can't be de-blurred)
redact {image_id:img, x:secretX, y:secretY, width:secretW, height:secretH,
        mode:"pixelate", block:14}                                → redacted
# or a clean black bar
redact {image_id:img, x:secretX, y:secretY, width:secretW, height:secretH,
        mode:"fill", color:"#111111"}                             → redacted
```

Get the region from Playwright (`boundingBox()` of the secret field), same as any
annotation. Redact **before** exporting to a lossy format.

## Beautify a hero screenshot

Turn a raw capture into a designed-looking figure — rounded corners, a soft drop
shadow, and breathing room — in one call. Trim any window chrome/whitespace first,
size it, then beautify **last** (it grows the canvas by `2·padding`):

```
open_image {path:"capture.png"}                       → raw
trim   {image_id:raw}                                 → tight     # drop uniform margins
resize {image_id:tight, width:1200}                   → sized
beautify {image_id:sized, padding:64, corner_radius:18,
          shadow_radius:28, shadow_opacity:0.35,
          background:"#0b1020"}                        → hero      # solid card…
# …or background:"transparent" for a rounded+shadowed PNG that drops onto any backdrop
export {image_id:hero, path:"hero.png"}
```

To point the eye at one control on a busy shot instead, `spotlight` it (dims the
rest): `spotlight {image_id:img, x, y, width, height, dim:0.6, corner_radius:12}`.

## Visual regression with diff

Check whether a UI changed between two captures, and get a number to assert on:

```
open_image {path:"baseline.png"}                      → base
open_image {path:"current.png"}                       → cur
diff {a_id:base, b_id:cur, view:"heatmap"}            → {image_id:"img-N", fraction_changed:0.012, max_difference:0.6, ...}
# fraction_changed ≈ 0 → no meaningful change; the heatmap handle shows WHERE it changed (glowing red)
export {image_id:"img-N", path:"regression.png"}
```

`difference`/`heatmap` need both shots the **same size** (resize one first if not);
`side_by_side` works for any sizes and is nice for a before/after in docs.

## Picture-in-picture zoom detail

Show a zoomed crop inset in a corner of the full shot:

```
open_image {path:"full.png"}                                   → full
crop   {image_id:full, x:dx, y:dy, width:dw, height:dh}        → detail       # the region of interest
resize {image_id:detail, width:520}                            → detailBig    # enlarge it
draw_rect {image_id:detailBig, x:0, y:0, width:519, height:.., stroke:"white", stroke_width:6} → framed
# place it bottom-right: x = fullW - 520 - 24, y = fullH - detailH - 24 (get sizes via image_info)
overlay {base_id:full, top_id:framed, x:.., y:..}              → pip
export  {image_id:pip, path:"with-detail.png"}
```

## Resizing screenshots for a doc width

Docs usually want a fixed content width (e.g. 1200px). Downscale with the default
Lanczos3 (sharp, no fringing) and let aspect ratio follow:

```
resize {image_id:img, width:1200}   # height auto — preserves aspect
```

Only pass both `width` and `height` when you deliberately want to force a shape (and
accept stretching). For crisp UI upscales (pixel art, icons) use
`filter:"nearest"`.

## Batch-processing many screenshots

The session's engine cache is shared across handles, so repeating the same operation
across similar images is cheap. Loop in your agent logic: for each file, `open_image`
→ apply the same edits → `export`. A common doc task is normalizing a folder of
captures to one width and format:

```
for each shot in folder:
    open_image {path:shot}         → h
    resize     {image_id:h, width:1200}  → h2
    export     {image_id:h2, path: shot with .png}
```

When the normalization is several steps, collapse them into one `run_pipeline` call
per image (same op names as the CLI pipeline):

```
for each shot in folder:
    open_image   {path:shot}       → h
    run_pipeline {image_id:h, pipeline:"[{\"op\":\"trim\"},{\"op\":\"resize\",\"width\":1200},{\"op\":\"beautify\",\"padding\":48}]"} → h2
    export       {image_id:h2, path: shot with .png}
```

For a single contact-sheet figure of the whole set, `open_image` them all and pass
every handle to one `collage`.
