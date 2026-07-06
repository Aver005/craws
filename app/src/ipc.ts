// Bridge to the Rust shell. Pixels arrive as ONE raw binary payload
// (tauri::ipc::Response) — never JSON. Layout: 20-byte LE header + RGBA8.
//   [0]  magic "CRAW" (0x57415243)
//   [4]  width   [8] height   [12] engine µs   [16] convert µs

import { invoke } from "@tauri-apps/api/core";

const MAGIC = 0x57415243;

export interface SourceInfo {
  width: number;
  height: number;
  loadMs: number;
}

export interface Frame {
  width: number;
  height: number;
  /** engine.run: hash derivation + cache + compute */
  engineMs: number;
  /** linear f32 tiles -> sRGB8 buffer */
  convertMs: number;
  /** everything else in the round-trip: serialize, IPC, deserialize, JS */
  bridgeMs: number;
  totalMs: number;
  bytes: number;
  pixels: Uint8Array;
}

export interface RenderParams {
  stops: number;
  grayscale: boolean;
}

export function loadSource(path: string | null): Promise<SourceInfo> {
  return invoke<SourceInfo>("load_source", { path });
}

export async function renderFrame(p: RenderParams): Promise<Frame> {
  const t0 = performance.now();
  const buf = await invoke<ArrayBuffer>("render", { stops: p.stops, grayscale: p.grayscale });
  const totalMs = performance.now() - t0;

  const dv = new DataView(buf);
  if (dv.getUint32(0, true) !== MAGIC) throw new Error("bad frame magic");
  const width = dv.getUint32(4, true);
  const height = dv.getUint32(8, true);
  const engineMs = dv.getUint32(12, true) / 1000;
  const convertMs = dv.getUint32(16, true) / 1000;
  return {
    width,
    height,
    engineMs,
    convertMs,
    bridgeMs: Math.max(0, totalMs - engineMs - convertMs),
    totalMs,
    bytes: buf.byteLength,
    pixels: new Uint8Array(buf, 20),
  };
}

export function reportBench(payload: unknown): Promise<void> {
  return invoke("report_bench", { payload: JSON.stringify(payload) });
}

/// Latest-wins render queue: at most one request in flight; while it flies,
/// only the newest requested params are kept. This is what turns a slider
/// drag into back-to-back frames instead of a backlog.
export function makeRenderQueue(onFrame: (f: Frame) => void, onError: (e: unknown) => void) {
  let pending: RenderParams | null = null;
  let inflight = false;

  return function request(p: RenderParams) {
    pending = p;
    if (inflight) return;
    inflight = true;
    (async () => {
      try {
        while (pending) {
          const cur = pending;
          pending = null;
          onFrame(await renderFrame(cur));
        }
      } catch (e) {
        onError(e);
      } finally {
        inflight = false;
      }
    })();
  };
}
