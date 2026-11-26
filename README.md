# PitchHelper

PitchHelper is a native Rust desktop app for singers. It listens to a selected
microphone, shows the closest note and tuning deviation, and plots pitch across
a moving four-bar 4/4 timeline.

## Run

Install the Rust toolchain, then run:

```sh
cargo run
```

Use **Start listening** to begin pitch tracking and select an input device from
the adjacent menu. Notes within 15 cents of pitch are green, flat notes are
red, and sharp notes are blue. Enharmonic spellings are shown for sharp notes.

Use **File > Open audio** to load WAV, MP3, FLAC, or OGG backing tracks. PitchHelper
attempts to detect a steady tempo and applies it to the timeline; use the tempo
slider to override the detected BPM. Play, pause, or stop the track from the
playback controls. Enable the metronome and adjust its volume under **Settings**.

The appearance setting follows the operating-system theme by default and can
be overridden with Light or Dark under **Settings > Appearance**.

## Build

Build a native release binary on each target platform with:

```sh
cargo build --release
```

On Linux, the system ALSA development library is required to build microphone
input support (for example, `libasound2-dev` on Debian-based distributions).
The resulting binary is in `target/release`.

## Test

```sh
cargo test
```
