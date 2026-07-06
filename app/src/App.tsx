// M2 spike screen: viewport + one exposure slider + live numbers overlay.
// Proves the whole authoritative path: slider -> Rust engine (warm cache)
// -> full-frame raw IPC -> WebGPU texture -> screen. Pan/zoom never leaves
// the GPU. An auto-bench runs once on startup and reports to stdout.

import { useCallback, useEffect, useRef, useState } from "react";
import { loadSource, makeRenderQueue, reportBench, type Frame } from "./ipc";
import { Viewport, type View } from "./renderer";

interface Stats {
  drawFps: number;
  pipeFps: number;
  engineMs: number;
  convertMs: number;
  bridgeMs: number;
  totalMs: number;
  mb: number;
}

const ZERO: Stats = { drawFps: 0, pipeFps: 0, engineMs: 0, convertMs: 0, bridgeMs: 0, totalMs: 0, mb: 0 };

export default function App() {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const viewportRef = useRef<Viewport | null>(null);
  const viewRef = useRef<View>({ zoom: 1, panX: 40, panY: 40, previewStops: 0, previewGray: false });
  const requestRef = useRef<((p: { stops: number; grayscale: boolean }) => void) | null>(null);
  const frameTimesRef = useRef<number[]>([]);
  const statsRef = useRef<Stats>({ ...ZERO });
  // 'optimistic' = pointwise ops in-shader (production design); 'authoritative'
  // = round-trip the engine every tick (the naive baseline, kept for comparison)
  const modeRef = useRef<"optimistic" | "authoritative">("optimistic");

  const [status, setStatus] = useState("loading 24MP sample…");
  const [error, setError] = useState<string | null>(null);
  const [source, setSource] = useState<{ width: number; height: number } | null>(null);
  const [stats, setStats] = useState<Stats>(ZERO);
  const [stops, setStops] = useState(0);
  const [grayscale, setGrayscale] = useState(false);
  const [mode, setMode] = useState<"optimistic" | "authoritative">("optimistic");
  const [benchResult, setBenchResult] = useState<string | null>(null);
  const benchBusyRef = useRef(false);

  const onFrame = useCallback((f: Frame) => {
    viewportRef.current?.upload(f.pixels, f.width, f.height);
    // the uploaded frame is authoritative → drop any in-shader preview transform
    viewRef.current.previewStops = 0;
    viewRef.current.previewGray = false;
    const now = performance.now();
    const times = frameTimesRef.current;
    times.push(now);
    while (times.length > 0 && now - times[0] > 1000) times.shift();
    statsRef.current = {
      ...statsRef.current,
      pipeFps: times.length,
      engineMs: f.engineMs,
      convertMs: f.convertMs,
      bridgeMs: f.bridgeMs,
      totalMs: f.totalMs,
      mb: f.bytes / (1024 * 1024),
    };
  }, []);

  // ── one-shot init: viewport, source, first frame, rAF loop, auto-bench ──
  useEffect(() => {
    const canvas = canvasRef.current!;
    let raf = 0;
    let disposed = false;

    (async () => {
      try {
        const vp = await Viewport.create(canvas);
        if (disposed) return;
        viewportRef.current = vp;
        const src = await loadSource(null);
        if (disposed) return;
        setSource(src);
        setStatus("");

        const request = makeRenderQueue(onFrame, (e) => setError(String(e)));
        requestRef.current = request;
        request({ stops: 0, grayscale: false });

        // draw loop: measures presentation fps (pan/zoom smoothness)
        let last = performance.now();
        const drawTimes: number[] = [];
        const loop = () => {
          const now = performance.now();
          drawTimes.push(now - last);
          last = now;
          if (drawTimes.length > 60) drawTimes.shift();
          const avg = drawTimes.reduce((a, b) => a + b, 0) / drawTimes.length;
          statsRef.current.drawFps = avg > 0 ? 1000 / avg : 0;
          vp.draw(viewRef.current);
          setStats({ ...statsRef.current });
          raf = requestAnimationFrame(loop);
        };
        raf = requestAnimationFrame(loop);

        // headless auto-bench: measure BOTH paths back-to-back, report each to stdout
        setTimeout(async () => {
          modeRef.current = "authoritative";
          setMode("authoritative");
          await runBench();
          // reset the base to neutral, then measure the optimistic in-shader path
          request({ stops: 0, grayscale: false });
          await new Promise((r) => setTimeout(r, 250));
          modeRef.current = "optimistic";
          setMode("optimistic");
          viewRef.current.previewStops = 0;
          await runBench();
        }, 1500);
      } catch (e) {
        setError(String(e));
      }
    })();

    return () => {
      disposed = true;
      cancelAnimationFrame(raf);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // ── auto-bench: 3s of continuous exposure sweeping ──
  // authoritative = engine round-trip per frame (bridge-bound); optimistic =
  // in-shader uniform sweep (draw-rate). Same gesture, both numbers.
  const runBench = useCallback(async () => {
    if (benchBusyRef.current || !requestRef.current) return;
    benchBusyRef.current = true;
    setBenchResult(null);
    const DURATION = 3000;
    const t0 = performance.now();
    let frames = 0;

    if (modeRef.current === "authoritative") {
      let engineSum = 0;
      let convertSum = 0;
      let bridgeSum = 0;
      await new Promise<void>((done) => {
        const queue = makeRenderQueue(
          (f) => {
            onFrame(f);
            frames += 1;
            engineSum += f.engineMs;
            convertSum += f.convertMs;
            bridgeSum += f.bridgeMs;
            const t = performance.now() - t0;
            if (t >= DURATION) return done();
            const s = Math.sin(t / 250) * 1.5;
            setStops(s);
            queue({ stops: s, grayscale: false });
          },
          () => done(),
        );
        queue({ stops: 0.1, grayscale: false });
      });
      const secs = (performance.now() - t0) / 1000;
      await finishBench({
        kind: "slider_drag_authoritative",
        frames,
        seconds: +secs.toFixed(2),
        fps: +(frames / secs).toFixed(1),
        avg_engine_ms: +(engineSum / frames).toFixed(2),
        avg_convert_ms: +(convertSum / frames).toFixed(2),
        avg_bridge_ms: +(bridgeSum / frames).toFixed(2),
      });
    } else {
      // optimistic: mutate the uniform each animation frame, no bridge traffic
      await new Promise<void>((done) => {
        const step = () => {
          const t = performance.now() - t0;
          const s = Math.sin(t / 250) * 1.5;
          viewRef.current.previewStops = s;
          setStops(s);
          frames += 1;
          if (t >= DURATION) return done();
          requestAnimationFrame(step);
        };
        requestAnimationFrame(step);
      });
      viewRef.current.previewStops = stops;
      const secs = (performance.now() - t0) / 1000;
      await finishBench({
        kind: "slider_drag_optimistic",
        frames,
        seconds: +secs.toFixed(2),
        fps: +(frames / secs).toFixed(1),
        bridge_calls: 0,
      });
    }
  }, [onFrame, stops]);

  const finishBench = useCallback(async (result: Record<string, unknown>) => {
    result.draw_fps = +statsRef.current.drawFps.toFixed(0);
    setBenchResult(`${result.fps} fps (${result.frames} frames / ${result.seconds}s)`);
    benchBusyRef.current = false;
    try {
      await reportBench(result);
    } catch {
      /* dev-in-browser: no tauri */
    }
  }, []);

  // ── input: slider behaviour depends on mode ──
  // optimistic → mutate the shader uniform only (no bridge); reconcile once on release.
  // authoritative → round-trip the engine on every change (the slow baseline).
  const onSlider = (v: number, gray = grayscale) => {
    setStops(v);
    if (modeRef.current === "optimistic") {
      viewRef.current.previewStops = v;
      viewRef.current.previewGray = gray;
    } else {
      requestRef.current?.({ stops: v, grayscale: gray });
    }
  };

  // pointer-up after an optimistic drag: bake the authoritative frame once
  const commitPreview = () => {
    if (modeRef.current !== "optimistic") return;
    requestRef.current?.({ stops, grayscale });
  };

  const dragRef = useRef<{ x: number; y: number } | null>(null);
  const onPointerDown = (e: React.PointerEvent) => {
    (e.target as Element).setPointerCapture(e.pointerId);
    dragRef.current = { x: e.clientX, y: e.clientY };
  };
  const onPointerMove = (e: React.PointerEvent) => {
    if (!dragRef.current) return;
    const dpr = window.devicePixelRatio || 1;
    viewRef.current.panX += (e.clientX - dragRef.current.x) * dpr;
    viewRef.current.panY += (e.clientY - dragRef.current.y) * dpr;
    dragRef.current = { x: e.clientX, y: e.clientY };
  };
  const onPointerUp = () => (dragRef.current = null);
  const onWheel = (e: React.WheelEvent) => {
    const v = viewRef.current;
    const dpr = window.devicePixelRatio || 1;
    const mx = e.clientX * dpr;
    const my = e.clientY * dpr;
    const factor = Math.exp(-e.deltaY * 0.0012);
    const zoom = Math.min(8, Math.max(0.05, v.zoom * factor));
    const k = zoom / v.zoom;
    v.panX = mx - (mx - v.panX) * k;
    v.panY = my - (my - v.panY) * k;
    v.zoom = zoom;
  };

  const num = (v: number, digits = 1) => v.toFixed(digits);

  return (
    <div className="relative h-full w-full">
      <canvas
        ref={canvasRef}
        className="h-full w-full cursor-grab active:cursor-grabbing"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onWheel={onWheel}
      />

      {/* stats overlay */}
      <div className="absolute left-4 top-4 w-72 rounded-xl border border-neutral-800 bg-neutral-950/85 p-4 font-mono text-xs backdrop-blur">
        <div className="mb-2 flex items-baseline justify-between">
          <span className="text-sm font-semibold tracking-wide text-neutral-100">craws</span>
          <span className="text-neutral-500">M2 bridge spike</span>
        </div>

        {error ? (
          <div className="whitespace-pre-wrap text-red-400">{error}</div>
        ) : (
          <>
            <div className="grid grid-cols-2 gap-x-3 gap-y-1 text-neutral-300">
              <span className="text-neutral-500">source</span>
              <span>{source ? `${source.width}×${source.height}` : status}</span>
              <span className="text-neutral-500">draw fps</span>
              <span>{num(stats.drawFps, 0)}</span>
              <span className="text-neutral-500">pipeline fps</span>
              <span>{num(stats.pipeFps, 0)}</span>
              <span className="text-neutral-500">engine</span>
              <span>{num(stats.engineMs)} ms</span>
              <span className="text-neutral-500">convert</span>
              <span>{num(stats.convertMs)} ms</span>
              <span className="text-neutral-500">bridge</span>
              <span>{num(stats.bridgeMs)} ms</span>
              <span className="text-neutral-500">round-trip</span>
              <span>{num(stats.totalMs)} ms</span>
              <span className="text-neutral-500">frame</span>
              <span>{num(stats.mb)} MB</span>
            </div>

            <div className="mt-3 border-t border-neutral-800 pt-3">
              <label className="mb-1 flex justify-between text-neutral-400">
                <span>exposure</span>
                <span>{stops >= 0 ? "+" : ""}{num(stops, 2)} ev</span>
              </label>
              <input
                type="range"
                min={-3}
                max={3}
                step={0.01}
                value={stops}
                onChange={(e) => onSlider(Number(e.target.value))}
                onPointerUp={commitPreview}
                className="w-full accent-amber-500"
              />
              <label className="mt-2 flex items-center gap-2 text-neutral-400">
                <input
                  type="checkbox"
                  checked={grayscale}
                  onChange={(e) => {
                    setGrayscale(e.target.checked);
                    onSlider(stops, e.target.checked);
                  }}
                  className="accent-amber-500"
                />
                grayscale
              </label>
            </div>

            <div className="mt-3 border-t border-neutral-800 pt-3">
              <div className="mb-1 text-neutral-500">preview mode</div>
              <div className="flex overflow-hidden rounded-md border border-neutral-800">
                {(["optimistic", "authoritative"] as const).map((m) => (
                  <button
                    key={m}
                    onClick={() => {
                      modeRef.current = m;
                      setMode(m);
                      if (m === "authoritative") {
                        viewRef.current.previewStops = 0;
                        requestRef.current?.({ stops, grayscale });
                      } else {
                        viewRef.current.previewStops = stops;
                        viewRef.current.previewGray = grayscale;
                      }
                    }}
                    className={
                      "flex-1 px-2 py-1 transition-colors " +
                      (mode === m
                        ? "bg-amber-600/90 font-semibold text-neutral-950"
                        : "bg-neutral-900 text-neutral-400 hover:bg-neutral-800")
                    }
                  >
                    {m === "optimistic" ? "optimistic (shader)" : "authoritative (bridge)"}
                  </button>
                ))}
              </div>
            </div>

            <div className="mt-3 flex items-center gap-2">
              <button
                onClick={() => void runBench()}
                className="rounded-md bg-neutral-800 px-3 py-1 font-semibold text-neutral-100 transition-colors hover:bg-neutral-700"
              >
                bench 3s
              </button>
              <span className="text-neutral-400">{benchResult ?? ""}</span>
            </div>
            <div className="mt-2 text-[10px] leading-4 text-neutral-600">
              drag to pan · wheel to zoom (GPU-only) · optimistic keeps the bridge idle during drag
            </div>
          </>
        )}
      </div>
    </div>
  );
}
