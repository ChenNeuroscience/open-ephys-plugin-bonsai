# Open Ephys (v1) Plugin - Camera

An Open Ephys source plugin that tracks an animal's head LEDs from an overhead camera and records their
positions as continuous channels alongside your ephys data. It's inspired by the kind of pipeline you'd
build in [Bonsai](https://bonsai-rx.org/), but it's a small, opinionated tracker written in Rust and runs
inside Open Ephys, with its own live-view window.

![The tracker window: arena with a square crop, two calibration rulers and the mouse's LEDs circled](docs/screenshot.png)

## What it does

Each camera frame goes through: **crop → threshold → connected regions → two largest regions**. The
centroids and areas of those two regions (normally the big and small headstage LEDs) become one sample.

| Channel | Contents |
|---|---|
| `BONS1` | camera timestamp, seconds since the first frame |
| `BONS2`, `BONS3` | x, y of the largest region (pixels) |
| `BONS4`, `BONS5` | x, y of the second largest region |
| `BONS6`, `BONS7` | areas of the two regions (pixels) |

A missing region gives NaN coordinates and zero area. The channel names are kept from earlier versions of this
plugin so existing analysis code keeps working.

While acquisition runs, a tracker window shows the live video with:

- **Camera**: frame rate, exposure and gain. Exposure and gain can be left as the camera has them
  (e.g. from SpinView), automatic, or manual. The frame rate is also the Open Ephys stream's sample rate,
  so a change takes effect from the next acquisition. If the camera won't take a frame rate (some models,
  or when it's configured elsewhere), **Detect** sets it to the rate the camera is actually delivering.
- **Crop**: none, circle, square or rectangle. Drag to move, use the handles to resize, and use the stalk to
  rotate (shift snaps to 15°). The eye shows the whole frame with the crop outline, or hides it and masks
  out everything outside the crop.
- **Detection**: brightness threshold and min/max region area. With a crop, **Near edge** sets a higher
  threshold within a band inside the boundary (shown dashed), so LED reflections off the walls don't
  count. The eye highlights above-threshold pixels and circles the two detected regions.
- **Calibration**: rulers you drag onto things of known size and give a length in cm. The mean pixels per
  metre is shown in the panel and over the video, even while the rulers are hidden.
- Below the video: a chart of the two LEDs' areas over the last 10 s (to check they stay distinct, e.g.
  while adjusting zoom and focus), plus a warning if frames arrive at a different rate from the stream's.
- Stats: frame rate, processing time, dropped frames.

The eye and padlock sit in each section's header. The padlock locks that section's controls and stops its
overlay being dragged on the video. While Open Ephys is recording, all settings are locked.

These settings are saved with the Open Ephys signal chain. If you close the window during acquisition,
**Open window** in the plugin's editor brings it back.

> **macOS:** the tracker window can't open inside Open Ephys on macOS (only the main thread can create
> windows there, and Open Ephys owns it), so the plugin runs without one. Use `tracker-dev` (below) to
> see the window. Windows and Linux are fine.

### Stream metadata (for record engines)

The tracker settings are attached as metadata to the data stream. Record engines can use them in file
headers:

| Identifier | Type | Contents |
|---|---|---|
| `camera_tracker.pixels_per_metre` | double | Mean of the calibration rulers; NaN if not calibrated |
| `camera_tracker.window` | int32 ×4 | Crop bounding box in frame pixels, clipped to the frame: min_x, min_y, max_x, max_y (whole frame without a crop) |
| `camera_tracker.crop` | char[128] | The crop's format, then its values separated by spaces (see below) |
| `camera_tracker.frame_size` | int32 ×2 | Frame width, height; 0 if not known yet |

`crop` is one of these, in frame pixels with angles in degrees clockwise on screen:

```
none()
circle(cx,cy,r) 320 240 200
square(cx,cy,w,deg) 320 240 400 45
rectangle(cx,cy,w,h,deg) 330 250 420 300 17.19
```

The fields are created when the signal chain updates. The plugin then updates their values in place
(checked every 250 ms, written only when they change) as the window changes, so they're current when
recording starts, unless a setting changed in the last quarter-second. They're never written while
recording: settings are locked then, and record engines read them from the record thread. Reading them from the stream:

```cpp
const DataStream* stream = nullptr; // the stream of the channel you identified as the camera's
for (const DataStream* s : getDataStreams())
    if (s->getStreamId() == getContinuousChannel(getGlobalIndex(0))->getStreamId()) stream = s;

int i = stream->findMetadata(MetadataDescriptor::DOUBLE, 1, "camera_tracker.pixels_per_metre");
double ppm = 0;
if (i >= 0) stream->getMetadataValue(i)->getValue<double>(ppm);
int32 window[4] = {};
int j = stream->findMetadata(MetadataDescriptor::INT32, 4, "camera_tracker.window");
if (j >= 0) stream->getMetadataValue(j)->getValue<int32>(static_cast<int32*>(window));
char crop[128] = {};
int k = stream->findMetadata(MetadataDescriptor::CHAR, 128, "camera_tracker.crop");
if (k >= 0) stream->getMetadataValue(k)->getValue<char>(static_cast<char*>(crop));
```

Pass the template arguments explicitly as shown. Otherwise C++ can pick overloads Open Ephys doesn't
export.

Updating in place relies on downstream copies of a stream sharing its metadata value objects. That's how
Open Ephys v1 copies streams, but it isn't a documented guarantee.

### Cameras

- **spinnaker**: Teledyne FLIR cameras (Blackfly S, Chameleon, Grasshopper...) via FLIR's
  [Spinnaker SDK](https://www.teledynevisionsolutions.com/products/spinnaker-sdk/), the same SDK Bonsai's
  Spinnaker package uses. Install the SDK; the plugin finds it at runtime and builds don't need it. Leave
  **Serial** in the plugin's editor blank to use the first camera found. The plugin sets 8-bit mono
  output, camera-clock (chunk) timestamps, the frame rate, and exposure/gain if you change them in the
  tracker window. Everything else (trigger mode, region of interest...) stays as configured in SpinView.
- **fake**: frames from `tools/fake_camera.py` over TCP (**Address**).
- **synthetic**: a built-in test pattern.

The Spinnaker support has only had light testing with a real camera so far.

### Timestamps and sample quality

The stream starts when acquisition does. Until the camera's first frame arrives (opening a camera can
take a few hundred milliseconds), the plugin emits empty samples on the host clock. The first frame is
then placed at the time it arrived, and from there on the camera's own timestamps are used. The
timestamp channel is seconds since acquisition started.

Samples are "smoothed" onto the frame rate set in the tracker window. A sample more than half a period
early is dropped. If a sample is more than half a period late, the gap is filled with empty samples. Empty
samples have a normal timestamp, NaN coordinates and zero areas, the same as a frame where no LED was
seen.

The editor's quality widget shows the last 10 seconds:
- **green:** samples used; a full bar is on time, shorter bars are slightly early or late;
- **white:** empty, gap-filled samples (including the start-up ones);
- **red:** dropped samples.

This reflects the camera's timestamps, not arrival times, so it gives internal consistency rather than
precise alignment with other data sources. The first frame's arrival latency, typically a few
milliseconds, becomes a constant offset.

The timestamp channel is a 32-bit float, which holds about three decimal places up to ~5 hours. Restart
acquisition at least that often.

## Installing

The plugin isn't part of the official Open Ephys set. Windows builds come from GitHub Actions:

- **Pull requests:** open the PR's workflow run and download the `open-ephys-plugin-camera` artifact.
- **master:** each push also creates a [release](https://github.com/ChenNeuroscience/open-ephys-plugin-bonsai/releases).

Copy `open-ephys-plugin-camera.dll` into `C:\Program Files\Open Ephys\plugins` and add the **Camera**
source to your signal chain.

## Development

You need CMake, a C++ toolchain, the [Open Ephys GUI source](https://github.com/open-ephys/plugin-GUI)
(built first), and Rust via [rustup](https://rustup.rs). CMake fetches
[Corrosion](https://github.com/corrosion-rs/corrosion) to build the Rust library.

```
Source/          C++ plugin: DataThread, editor, sample smoothing (SampleWriter)
tracker/         Rust library: cameras, pipeline, window (egui), C API (include/tracker.h)
tools/           fake_camera.py: a fake overhead camera over a 1x1 m arena with a mouse
PLAN.md          architecture and roadmap
```

### Without Open Ephys

```bash
python3 tools/fake_camera.py                      # fake camera on 127.0.0.1:27030 (standard library only)
cd tracker
cargo run --release --bin tracker-dev             # the tracker window, printing samples to stdout
cargo run --release --bin tracker-dev -- synthetic   # or without the fake camera
cargo run --release --bin tracker-dev -- spinnaker   # or a FLIR camera (needs the Spinnaker SDK)
cargo test --release -- --include-ignored         # tests, incl. end-to-end against fake_camera.py
```

The window tests compare against images in `tracker/tests/snapshots/`. Run with `UPDATE_SNAPSHOTS=1` to
accept intended changes. `cargo test --release readme_screenshot -- --ignored` regenerates
`docs/screenshot.png`.

### Building the plugin on macOS

```bash
# in the Open Ephys GUI's Build dir, first
cmake -G Xcode ..

# then in this repo's Build dir (set GUI_BASE_DIR to your GUI checkout)
GUI_BASE_DIR=../../plugin-GUI cmake -G Xcode ..
cmake --build . --config Release
cp -R Release/open-ephys-plugin-camera.bundle ~/Library/Application\ Support/open-ephys/plugins-api10/
```

macOS builds are for the host architecture only (Corrosion can't build universal binaries).

### Building on Windows

See [.github/workflows/windows.yml](.github/workflows/windows.yml) for the exact steps CI uses.
