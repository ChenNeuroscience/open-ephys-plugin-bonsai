# Plan: inline camera tracking (Rust) in place of Bonsai + OSC

Branch: `rust-tracker`

## Goal

Replace "Bonsai → OSC/UDP → OSCServer" with an in-process Rust library (`tracker/`) that
captures frames, finds the LEDs and hands samples straight to the DataThread. It also opens
its own window (live video, overlays, config knobs) while acquisition runs.

The output stays the same as today, so downstream consumers keep working:
`[timestamp, x1, y1, x2, y2, area1, area2]` (7 float32 channels, `BONS1..7`), with the
existing timestamp smoothing and the quality widget.

## Architecture

```
 Open Ephys process
 ┌──────────────────────────────────────────────────────────────────────┐
 │ C++ plugin (DataThread)                                              │
 │   startAcquisition → tracker_start(config_json)                      │
 │   updateBuffer     → tracker_poll(..., timeout) → SampleWriter       │
 │                      (smoothing + QualityInfo + DataBuffer, as now)  │
 │   stopAcquisition  → tracker_stop                                    │
 │                         │ C ABI (tracker/include/tracker.h)          │
 │ Rust staticlib ─────────┴──────────────────────────────────────────  │
 │   capture thread:  Camera::next_frame → Pipeline::process            │
 │                    → sample channel (polled by C++)                  │
 │                    → latest-frame slot (read by GUI)                 │
 │   gui thread:      eframe/egui window; reads latest frame + blobs,   │
 │                    writes config (threshold etc.) via shared state   │
 └──────────────────────────────────────────────────────────────────────┘
```

* **Data path is pull, not push.** The C++ side calls `tracker_poll` from `updateBuffer()`.
  Rust never calls into C++ or JUCE, so there's no FFI callback on a foreign thread.
  `poll` blocks with a timeout, so DataThread's busy `run()` loop doesn't spin.
* **Config** is JSON across the FFI boundary (one string in, `serde` on the Rust side),
  so adding knobs doesn't change the C ABI.
* **The GUI thread is persistent.** winit allows only one event loop per process. The GUI
  thread is created on the first start, lives as long as the library, and runs
  `eframe::run_native` again (re-entrant on the same thread) each time acquisition starts.
  Stopping acquisition closes the window.

### ⚠ macOS caveat

winit (and AppKit in general) can only create windows on the **main thread** of the
process, and in Open Ephys JUCE owns that thread. Windows doesn't have this restriction
(`with_any_thread(true)`), so the in-process window works on Windows (the rig). On macOS:

* the plugin runs **headless** in Open Ephys (capture and samples work, no window);
* `cargo run --bin tracker-dev` runs the same capture, pipeline and GUI as a standalone
  app (GUI on main thread). That's the main dev loop on the Mac anyway.

If an in-process window on macOS ever matters, there are two options: (a) run the GUI as a
child process with frames shared via shared memory, or (b) drive winit's `pump_events`
from the JUCE message thread. Both are deferred.

Crash isolation: a panic in a Rust thread is caught and reported as an error status (no
unwinding across FFI). A segfault in a camera SDK would still take down Open Ephys. If
that turns out to be a real problem, we'd move capture out-of-process. The crate split
below keeps that option cheap.

## Choices

| Concern | Choice | Why |
|---|---|---|
| GUI (outer) | **egui / eframe** (wgpu backend) | The most widely used Rust GUI, maintained by Rerun, whose app (docked panes, GPU scientific viewers) is close to what we want long-term. Immediate mode, fast to iterate. `egui_dock` for panes. We write the node graph ourselves on egui's painter rather than depend on a small crate like `egui-snarl`. Alternatives considered: iced (Elm-style, awkward for dense tool UIs), Slint (declarative, poor fit for custom canvases), gpui (API churn), Qt/cxx-qt (long-lived but a heavy C++ toolchain and licensing). |
| GUI (GPU) | **wgpu + WGSL** via `egui_wgpu` paint callbacks | Custom shaders render straight into any egui panel in the same GPU context (video, LUTs, waveforms, heatmaps). This is the "two libraries that play nice" layer. |
| Image processing | **image + imageproc** | Polygon mask (`draw_polygon_mut`), threshold, connected components. Pipeline is small and opinionated; no OpenCV (painful to build, esp. Windows). |
| Camera | `Camera` trait with backends | See below. |
| Build | **Corrosion** (CMake ↔ Cargo) | Builds the staticlib from CMake and handles the native link libs on MSVC and Xcode. |

### Camera backends

1. `synthetic`: in-process moving two-LED scene. No dependencies; used for tests.
2. `fake`: TCP client that reads raw frames from `tools/fake_camera.py` (pure-stdlib
   Python, simulates an animal with two LEDs, same path model as `dummy-bonsai.py`).
   Wire format: `b"FRM1"`, u32 width, u32 height, u64 frame id, f64 timestamp (s), then
   `width*height` gray8 bytes, all little-endian.
3. *(phase 2)* `webcam`: `nokhwa` (UVC/AVFoundation/MSMF). Covers Bonsai's
   `CameraCapture` and lets you test with the Mac's built-in camera.
4. `spinnaker`: the rig camera. The Bonsai workflow used `SpinnakerCapture`, i.e. a Teledyne FLIR camera.
   We use the Spinnaker SDK's C API, loaded at runtime, rather than pure-Rust GenICam (`cameleon`). On
   Windows the FLIR driver owns USB3 Vision devices, so `cameleon` (libusb) would need a driver swap that
   breaks SpinView and Bonsai. Hardware TTL triggering stays out of scope; the camera's own trigger
   settings are left alone, and frames carry the camera-clock chunk timestamp (as `ChunkData.Timestamp`
   did in Bonsai).

## Pipeline (mirrors the Bonsai workflow)

`Grayscale → CropPolygon(mask, ToZero) → Threshold → FindContours(External, MinArea) →
BinaryRegionAnalysis → SortBinaryRegions (area desc) → top 2 centroids + areas`

Implementation: threshold inside the polygon mask, then 8-connected components, then
per-label area and centroid, filter by `min_area`/`max_area`, sort by area, and take 2.
One small difference from Bonsai's External contours: holes inside a blob aren't counted
in its area. That doesn't matter for LED blobs. Missing blobs give `NaN` coordinates and
0 area. (Bonsai currently writes hard-coded 8/4 areas; we write the real pixel counts.)

Config knobs (JSON, editable live in the GUI): `threshold`, `min_area`, `max_area`, `crop`
(circle / square / rectangle, rotatable), plus calibration rulers. Camera selection comes from the
plugin's editor.

## Phases

**Phase 1: proof of concept (this branch, now)**
- [x] Branch, plan
- [x] `tracker/` crate: config, `Camera` trait + synthetic + fake TCP, pipeline, GUI, C ABI
- [x] `tracker-dev` standalone binary (Mac dev loop)
- [x] `tools/fake_camera.py` (+ `cargo test -- --ignored` end-to-end test against it)
- [x] C++: extract the smoothing/quality logic from `OSCServer` into `SampleWriter`
      (fixing the out-of-bounds NaN gap fill, the always-zero buffer timestamps and gap
      rounding); replace OSC with the tracker; new editor params; status line in the editor
- [x] CMake: Corrosion, link the staticlib (macOS builds are now single-arch, since Corrosion
      can't do universal binaries)
- [x] Unit tests for the pipeline (synthetic frames with known blobs)
- [x] Fake scene with perspective: 1x1 m arena, 45 cm walls, mouse with headstage LEDs, tether to a
      commutator 2 m up
- [x] Crop modes (circle / square / rectangle) with on-image move/resize/rotate handles
- [x] Rulers with physical lengths giving mean px/m, which stays visible when rulers are hidden
- [x] Window state (threshold, crop, rulers) round-trips via `tracker_state_json` into a hidden
      `tracker_state` plugin parameter, so it's saved with the signal chain (`tracker-dev` uses a file)
- [x] Offscreen snapshot test of the window (`egui_kittest`, `tests/snapshots/`)
- [ ] Try it in Open Ephys on macOS (headless) and on the Windows rig (with window)
- [ ] Windows CI: install Rust in the workflow

**Phase 2: real cameras + config persistence**
- [x] Spinnaker backend: the SDK's C library is loaded at runtime with `libloading` (no SDK needed to
      build or in CI); signatures checked against Spinnaker.jl's generated bindings. Sets Mono8, chunk
      timestamps and frame rate; leaves exposure/gain/trigger as configured in SpinView.
      **Untested with real hardware**: first run on the rig with `tracker-dev -- spinnaker`.
- [x] Video drawn on the GPU (8-bit texture upload + WGSL shader for mask/highlight) instead of a
      per-frame CPU colour conversion
- camera settings (exposure, gain) in the tracker window
- webcam backend (nokhwa), if wanted
- polygon crop (for arenas that aren't circles or rectangles), if needed

**Phase 3: performance and timing**
- avoid per-frame allocation (reuse label buffers); ROI bounding box of the polygon
- frame-drop and latency stats in the GUI; camera timestamp vs host clock
- optional raw video recording alongside Open Ephys recordings

**Phase 4: groundwork for the Rust Open Ephys**
- `egui_dock` workspace and a minimal node graph (`egui-snarl`) describing this pipeline
- split the crate into `tracker-core` (no GUI) and `tracker-gui` once it's worth it

## Open questions
- Exact camera model, interface (USB3/GigE), resolution and frame rate on the rig.
- Should we keep OSC as a fallback source during the transition? Currently: no. Master
  still has it.
- Does anything downstream (e.g. the LTX plugin) depend on the `BONS*` channel names or
  on the area values being 8/4?
