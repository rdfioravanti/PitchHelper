# PitchHelper

**PitchHelper** is a native desktop app that acts as a real-time pitch coach for
singers. It listens to your microphone, tells you the closest note and how far off
you are in **cents**, and draws your pitch across a moving four-bar (4/4) timeline —
so you can hear and *see* your tuning as you sing, practice against a backing track,
and watch your range and tessitura build up over time.

It is written in Rust with no web runtime, and builds to a single native binary.

|              |                                             |
| ------------ | ------------------------------------------- |
| **Author**   | Richard Fioravanti                          |
| **Version**  | 0.1.0                                       |
| **Language** | Rust (edition 2024)                         |
| **Platforms**| Windows · macOS · Linux                     |

<!-- TODO: drop a screenshot here, e.g.  ![PitchHelper](docs/screenshot.png) -->

## Contents

- [What it does](#what-it-does)
- [Features](#features)
- [Getting started](#getting-started)
- [Using the app](#using-the-app)
  - [Microphone & live pitch](#microphone--live-pitch)
  - [Backing track & playback](#backing-track--playback)
  - [The pitch graph](#the-pitch-graph)
  - [Practice tools](#practice-tools)
  - [Vocal range & tessitura](#vocal-range--tessitura)
  - [Settings & appearance](#settings--appearance)
- [Defaults & reference](#defaults--reference)
- [Project layout](#project-layout)
- [Under the hood](#under-the-hood)
- [Building from source](#building-from-source)
- [Running the tests](#running-the-tests)
- [Copyright & license](#copyright--license)

## What it does

Sing a note, and PitchHelper does three things at once:

1. **Names the note** — the closest pitch and your deviation from it in cents.
2. **Colors the tuning** — green when you are in tune (within 15 cents), red when
   flat, blue when sharp.
3. **Draws it over time** — your pitch is plotted across a moving four-bar 4/4
   timeline, optionally in sync with a backing track and metronome.

Around that core it adds practice aids (key/scale highlighting, a reference tone,
guided warm-ups, and a pitch-echo game) and quietly tracks your **vocal range** and
**tessitura** so you can see your own progress.

## Features

**Live pitch**

- Real-time pitch detection from any input device, with the closest note and a
  cent-accurate deviation readout.
- Color-coded tuning: green in tune (≤ 15 cents), red flat, blue sharp. Note names
  use ASCII accidentals (e.g. `C#`, `Db`), including enharmonic spellings.
- Live input level meter with a clip warning, plus a **Pitch Sensitivity** control
  for quieter singing.
- Pitch width, vibrato, and stability readout while you hold a note.

**Play along**

- Load **WAV, MP3, FLAC, or OGG** backing tracks (the picker and analysis run in the
  background, so the UI stays responsive).
- Auto-detected tempo applied to the timeline, with a slider to override it.
- A metronome with its own volume.
- A seekable amber waveform with A/B handles for looping a section.

**Practice**

- Key and scale highlighting — in-scale notes are tinted and chord tones are
  emphasized (major, minor, and both pentatonic scales).
- A reference tone on the key's root or fifth, shown as a target line.
- **Warm-Up**: five-tone scales, octaves, arpeggios, or sirens from the shared root,
  advancing by a semitone after each pass.
- **Pitch Echo**: a 3–2–1 countdown, a three-note phrase to listen to, then a scored
  response.

**Track yourself**

- Vocal range and tessitura, accumulated from your stable notes and saved locally so
  your progress persists between sessions.

## Getting started

You need the [Rust toolchain](https://rustup.rs/). With it installed:

```sh
cargo run
```

That's the whole installation — PitchHelper ships as one native binary and has no
runtime dependencies. Platform notes:

- **Windows / macOS** — no extra packages; microphone input uses the system audio
  stack (WASAPI / CoreAudio).
- **Linux** — microphone input needs the ALSA development package, e.g.
  `libasound2-dev` on Debian-based distributions, and the app runs against your
  display server.

## Using the app

### Microphone & live pitch

Click **Start Listening** to begin pitch tracking, and pick your input device from
the adjacent menu. The list refreshes automatically when audio devices change; use
the **Refresh Microphones** button to rescan immediately.

The **Pitch Sensitivity** slider under the microphone lowers the detection threshold
so the app can hear quieter singing, and the meter shows your input level and warns
you when the signal clips.

You'll see the closest note and your deviation in cents. Notes within **15 cents** of
pitch are green, flat notes are red, and sharp notes are blue. Enharmonic spellings
use ASCII sharps and flats.

### Backing track & playback

Use **File → Open audio** to load a **WAV, MP3, FLAC, or OGG** backing track. The
file picker opens immediately and the track's steady tempo is analyzed in the
background, so the interface stays responsive while the picker is open and the file
loads. The detected tempo is applied to the timeline; use the tempo slider to
override it.

Play, pause, or stop the track from the playback controls and adjust its volume with
the track volume slider (default 50%). Enable the metronome and adjust its volume
with the **Metronome** controls on the main screen.

With a track loaded, the playhead and metronome follow playback — they pause with the
track, and **Stop** resets the playhead to the beginning. **Clear Track** returns the
playhead to free-running mode.

The contrasting amber waveform above the pitch graph shows the track's relative
volume. Hover to preview a timestamp, then click or drag to seek; the current and
total track times appear beside the name. The A/B handles default to the full track;
drag them to loop a section, or right-click the waveform to reset either handle or
both. Seeking outside the visible four-measure cycle clears the pitch trace.

### The pitch graph

Use **Zoom**, **LOW**, and **HIGH** beside the tempo control to shape the graph.

- **LOW / HIGH** set the graph's boundaries. The default **1×** view zooms into those
  bounds; **0.5×** shows the full range.
- In free singing, the graph follows your live pitch smoothly as it nears the edge,
  without moving beyond LOW/HIGH.
- During warm-ups and pitch echo it centers on the exercise's target notes.

The selectable range includes **C1** and extends above **C6**. Note names use ASCII
accidentals (e.g. `C#`, `Db`).

### Practice tools

Choose a **Key** and **Scale** in the Practice controls to highlight the in-scale
note rows; chord tones are highlighted more strongly. Their subtle **4% default
opacity** can be adjusted in **Settings**.

The **Reference Tone** control plays the selected key's **root or fifth**, shown as a
yellow target line. The key's octave is chosen alongside its root and is shared by
the reference tone, warm-ups, and pitch echo.

- **Warm-Up** plays five-tone **scales, octaves, arpeggios, or sirens** from the
  shared root (C1–C6) and moves up by a semitone after each pass. Changing the root,
  octave, scale, or pattern stops the current warm-up.
- **Pitch Echo** shows a **3–2–1 countdown** in the graph, plays a three-note phrase,
  then scores the note you sing back.

Hold a note to see the pitch-width and vibrato/stability readout.

### Vocal range & tessitura

Stable notes contribute to your **vocal range** and **tessitura** display. That
history is saved locally in your user's application data directory, so it carries
over between sessions.

### Settings & appearance

The appearance follows the operating-system theme by default and can be overridden
with **Light** or **Dark** under **Settings → Appearance**.

## Defaults & reference

| Setting                        | Default                     | Notes                                                    |
| ------------------------------ | --------------------------- | -------------------------------------------------------- |
| In-tune tolerance              | 15 cents                    | Green band; outside this is red (flat) or blue (sharp).  |
| Track volume                   | 50%                         | Playback volume, settable in the slider.                 |
| Scale-row highlight opacity    | 4%                          | In-scale tint; chord tones are stronger.                 |
| Reference tone                 | Root or fifth               | Shown as a yellow target line.                           |
| Pitch detection range          | 25 – 2,500 Hz               | Autocorrelation over the mic buffer.                     |
| Note range                     | C1 to above C6              | Selectable graph and practice range.                     |
| Graph zoom                     | 1× (LOW/HIGH bounds)        | 0.5× shows the full range.                               |
| Metronome                      | Off                         | Enable with the Metronome controls.                      |
| Pitch Echo phrase              | 3 notes                     | After a 3–2–1 countdown.                                 |
| Appearance                     | OS theme                    | Override with Light/Dark in Settings.                    |

## Project layout

| File          | Role                                                                                          |
| ------------- | --------------------------------------------------------------------------------------------- |
| `src/main.rs` | The egui/eframe UI shell — menu bar, pitch readout, graph, playback & practice controls.      |
| `src/audio.rs`| Microphone capture (cpal): device list, the capture thread, level/clip metering, state.       |
| `src/pitch.rs`| Pitch detection — autocorrelation across 25–2,500 Hz, returning frequency + MIDI.              |
| `src/music.rs`| Track analysis (symphonia) — decode to detect tempo (BPM), duration, and a waveform.          |
| `src/playback.rs` | Playback (rodio) — decode, play/pause/seek/volume, plus reference-tone generation.       |
| `src/practice.rs` | Scales, chord tones, warm-ups, pitch echo, and vocal-range/tessitura history.             |

## Under the hood

The audio pipeline runs across a small set of focused modules:

1. **Capture** (`audio.rs`) pulls raw microphone samples on a background thread via
   **cpal**, measures RMS/peak level for the meter, and feeds the detector.
2. **Detection** (`pitch.rs`) runs an autocorrelation search over a 25–2,500 Hz window
   and returns a `PitchReading` (frequency + MIDI), from which the UI derives the note
   name and cent deviation.
3. **Analysis** (`music.rs`) decodes a backing track with **symphonia** (WAV/MP3/FLAC/OGG)
   to estimate its tempo and build a ~1,600-bin waveform, all off the UI thread.
4. **Playback** (`playback.rs`) uses **rodio** to decode and play the track, seek, and
   mix in the metronome and reference tone.
5. **Practice** (`practice.rs`) defines the scales (major, minor, and both pentatonics),
   their chord tones, the warm-up/pitch-echo logic, and the persisted range history.

**Dependencies:** `eframe`/`egui` (UI) · `cpal` (input) · `rodio` (output) ·
`symphonia` (decode/analysis) · `dark-light` (OS theme) · `rfd` (native file dialog).

## Building from source

Build a native release binary on your target platform:

```sh
cargo build --release
```

On Linux, the system ALSA development library is required to build microphone input
support (for example, `libasound2-dev` on Debian-based distributions). The resulting
binary is in `target/release/`.

## Running the tests

```sh
cargo test
```

## Copyright & license

Copyright © 2026 Richard Fioravanti. *PitchHelper* is a working name for the project
and is not claimed as a registered trademark.

No open-source license is currently specified; the code is provided as-is. To request
a license or contribute, please get in touch.
