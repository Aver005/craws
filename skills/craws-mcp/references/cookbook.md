# craws MCP — cookbook

Concrete, end-to-end recipes. Coordinates are pixels (top-left origin). Every step
threads the returned `image_id` into the next call.

## Contents
- [Playwright → craws → Outline: annotated docs](#playwright--craws--outline-annotated-docs)
- [Getting element coordinates right (DPR / retina)](#getting-element-coordinates-right-dpr--retina)
- [Numbered-step callouts](#numbered-step-callouts)
- [Before / after plate](#before--after-plate)
- [Redacting secrets](#redacting-secrets)
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

Put a numbered badge next to each highlighted element. Until a text tool lands, use a
filled circle as the badge marker and let the surrounding doc carry the number, or
draw the marker and rely on ordering:

```
# highlight the target, then mark it with a solid dot the doc text refers to as "①"
draw_rect    {image_id:img, x:.., y:.., width:.., height:.., stroke:"#2563eb", stroke_width:4} → a
draw_ellipse {image_id:a, x:markX, y:markY, width:28, height:28, fill:"#2563eb"}               → b
```

When the text tool arrives, the number goes inside the circle at `(markX, markY)`.

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

Cover the region with an **opaque** rectangle. A translucent fill is not redaction —
the value shows through. There is no blur op yet; a solid bar is the safe choice.

```
draw_rect {image_id:img, x:secretX, y:secretY, width:secretW, height:secretH,
           fill:"#111111"}   → redacted
```

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

For a single contact-sheet figure of the whole set, `open_image` them all and pass
every handle to one `collage`.
