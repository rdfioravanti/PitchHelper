# PitchHelper

PitchHelper is a native Rust desktop app for singers. It listens to the default
microphone, shows the closest note and tuning deviation, and plots pitch across
a moving four-bar 4/4 timeline.

## Run

Install the Rust toolchain, then run:

```sh
cargo run
```

Allow microphone access when prompted. Use **Start listening** to begin pitch
tracking and adjust the tempo slider to set the timeline speed. Notes within
15 cents of pitch are green, flat notes are red, and sharp notes are blue.
Enharmonic spellings are shown for sharp notes.

The appearance menu follows the operating-system theme by default and can be
overridden with Light or Dark.

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
