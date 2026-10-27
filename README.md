# Open Ephys (v1) Plugin - Bonsai

> **`rust-tracker` branch:** the OSC/Bonsai input is replaced by an in-process Rust camera tracker
> (`tracker/`), which captures frames, finds the two LEDs and opens its own live-view window during
> acquisition (Windows; on macOS it runs headless, so use `tracker-dev`). See [PLAN.md](PLAN.md). Quick start:
>
> ```bash
> python3 tools/fake_camera.py                       # fake camera on 127.0.0.1:27030 (stdlib only)
> cd tracker && cargo run --release --bin tracker-dev  # standalone window + sample printout
> cargo test --release -- --include-ignored            # unit tests + end-to-end test vs fake_camera.py
> ```
>
> Building the plugin now also needs a Rust toolchain (https://rustup.rs); CMake fetches
> [Corrosion](https://github.com/corrosion-rs/corrosion) to drive cargo. The channel layout is unchanged.


<img width="291" height="157" alt="Screenshot 2025-07-11 at 00 42 37" src="https://github.com/user-attachments/assets/2f920dce-3caf-465e-9eb0-74358f9c16b2" />


This runs an OSC UDP Server (using an open ephys DataThread plugin). The Server expects messages that optionally start with a timestamp value, and then contain 1-8 float values
(you must specify the exact number of values in the interface and specify if there is a timetamp first or not). Each of the float values is given its own continuous channel.
You can view the raw values in openephys using the LFP viewer, though it's not going to be that interesting in that form. The timestamp is not plugged into the proper timestamp
machinery of openephys because that doesn't actually seem to be visible to the record engine downstream. Instead we put the timestamp into its own channel (the first channel,
before the other channels).

There is a slight problem with having a 32bit timestamp though:  32 bit floats can only just about represent 3 decimal places above 20,000 (5hs in seconds); at that point there are
about 5 representable values in the third decimal place for each value in the second decimal place. But hopefully that's enough here. The timestamp provided by Bonsai should be
a 64 bit double, but it will then be converted to a single (32bit) float here, with the first received timestamp treated as the zero point (which ensures that the number are small
so long as acquisition is restarted at least every 4hrs or so).

If you do provide a timestamp, the plugin is then able to "smooth" the samples to match the exact sample rate you stated. This means that if a given sample's timestamp is too early
it may get dropped entirely, and if it's very late NaNs will be used to fill the gap. The widget has a way of visualising this: dropped samples are shown in red; gaps filled are
shown in white; green is good samples, specifically a full green bar is a timestamp that is accurate with partial green bars indicating a slightly early or late sample.  To be clear
this is not referencing when the data was recieved, it's just looking at the value of the timestamps that were recieved relative to the first such timestamp and the stated sample rate.

The intention is that the data is coming from a camera via Bonsai, with the float values corresponding to some kind of x and y values - indeed we provide a python script here,
`dummy-bonsai.py` which can be used to simmulate an animal running around indefinitely with two leds on its head.

WARNING: this is not yet going to allow for any kind of precision in terms of timing latency relative to other data sources, it only provides internal consistency.

## Installing

This plugin is not part of the offical set of Open Ephys plugins, but you can easily install it yourself by downloading the `.dll` file from the [latest release](https://github.com/d1manson/open-ephys-plugin-bonsai/releases)
and then moving it into your `C:\Program Files\Open Ephys\plugins` folder (Mac/Linux binaries not provided here currently).

## Development

See the [Open Ephys Plugin API docs](https://open-ephys.github.io/gui-docs/Developer-Guide/Open-Ephys-Plugin-API/index.html) for info on how to develop a plugin
including links to the template repos. It's also worth looking at the source code for other plugins and the plugin-gui repo itself. A lot of what's been done here was
inspired by existing code.

As shown in `Source/OpenEphysLib` this is a data thread plugin, but it's worth reading the docs more widely to get better context.

I found on Mac that you can build using this:

```bash
brew install cmake

# run this is the main GUI's Build dir (need to do this first)
CMAKE_C_COMPILER=clang CMAKE_CXX_COMPILER=clang cmake -G "Xcode" ..

# run this in the root of this plugin (set the GUI_BASE_DIR appropriately)
GUI_BASE_DIR=../main-gui CMAKE_C_COMPILER=clang CMAKE_CXX_COMPILER=clang cmake -G "Xcode" .

# and then to actually build
cmake --build . --config Release -- -arch x86_64 &&
cp -R Release/open-ephys-plugin-bonsai.bundle /Users/<YOUR_MAC_USER_NAME_HERE>/Library/Application\ Support/open-ephys/plugins-api8/ 
```
