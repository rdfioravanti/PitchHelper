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
| **Platforms**| Windows · Linux · macOS                    |

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
- Optional microphone monitoring through the default audio output; headphones help
  prevent feedback.
- Pitch width, detected vibrato rate, and stability readout while you hold a note.

**Play along**

- Load **WAV, MP3, FLAC, or OGG** backing tracks (the picker and analysis run in the
  background, so the UI stays responsive).
- Auto-detected tempo applied to the timeline, with a slider to override it.
- A beat-synchronized metronome with its own volume and a higher-pitched accent on
  the first beat of each measure.
- A seekable amber waveform with A/B handles for looping a section.

**Practice**

- Key and scale highlighting — in-scale notes are tinted and chord tones are
  emphasized (major, minor, and both pentatonic scales).
- A reference tone on the key's root or fifth, shown as a target line.
- **Warm-Up**: five-tone scales, octaves, arpeggios, or sirens from the shared root,
  with one note per beat at the selected tempo, transposing by a semitone between
  passes across an octave.
- **Pitch Echo**: choose the same patterns as Warm-Up. Each sequence begins with a
  graph 3–2–1 countdown, followed by the phrase and a second 3–2–1 pause before you
  sing it back. Review your match score before choosing **Next Sequence**.

**Track yourself**

- Your detected note range and an estimated tessitura, saved locally so your
  progress persists between sessions. Tessitura is estimated from the central 80%
  of stable pitch samples; it describes observed stability, not subjective comfort.

## Getting started

You need the [Rust toolchain](https://rustup.rs/). With it installed:

```sh
cargo run
```

There is no installer; build separately on each target operating system. Platform
notes:

- **Windows** — microphone input uses the system audio stack (WASAPI).
- **Linux** — microphone input needs the ALSA development package, e.g.
  `libasound2-dev` on Debian-based distributions. ALSA and the system libraries for
  X11 or Wayland must also be available at runtime.
- **macOS** — microphone input uses CoreAudio. Intel and Apple Silicon builds are
  produced separately.

## Using the app

### Microphone & live pitch

Choose an input device from the menu beside **Start Listening**, then start pitch
tracking. The device list refreshes about every two seconds; use **Refresh
Microphones** to rescan immediately.

The **Pitch Sensitivity** slider under the microphone lowers the detection threshold
as sensitivity increases, which can help with quieter singing. The meter shows your
input level and warns when the signal clips.
When listening is active, enable **Monitor** beside the sensitivity control to play
the microphone through the default output device. Headphones are recommended to
avoid acoustic feedback.

You'll see the closest note and your deviation in cents. Notes within **15 cents** of
pitch are green, flat notes are red, and sharp notes are blue. Enharmonic spellings
use ASCII sharps and flats.

### Backing track & playback

Use **File → Open Audio** to load a **WAV, MP3, FLAC, or OGG** backing track. The
file picker and track analysis run in the background. A steady tempo is applied to
the timeline when detected; if no tempo is detected, or the estimate is off, set it
with the **Tempo** slider (40–240 BPM).

Play, pause, or stop the track from the playback controls and adjust its volume with
the track volume slider (default 50%). Enable the metronome and adjust its volume
with the **Metronome** controls on the main screen. Its quarter-note clicks follow
the timeline or playing track, with the higher-pitched click on each measure's first
beat.

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

- **LOW / HIGH** set the graph's boundaries. Zoom scales relative to the selected
  range: **0.5×** shows the full range, **1×** shows half, and **2×** shows a quarter.
- In free singing, the graph follows your live pitch smoothly as it nears the edge,
  without moving beyond LOW/HIGH.
- During warm-ups and pitch echo it centers on the exercise's target notes; the
  exercise playhead follows the phrase and response rather than the full four-bar
  cycle.

The default graph bounds are **C2–C6**. The LOW and HIGH menus can be adjusted
between **C0 and C9**. Note names use ASCII accidentals (e.g. `C#`, `Db`).

### Practice tools

Choose a **Key** and **Scale** in the Practice controls to highlight the in-scale
note rows; chord tones are highlighted more strongly. Available scales are Major,
Minor, Major Pentatonic, and Minor Pentatonic. The root-note menu lists every note
within the selected **LOW** and **HIGH** graph bounds. The subtle **4% default
opacity** can be adjusted in the **Settings → Highlight Opacity** side submenu.

The **Reference Tone** control plays the selected key's **root or fifth**, shown as a
yellow target line. The selected root note is shared by the reference tone,
warm-ups, and pitch echo.

- **Warm-Up** plays five-tone **scales, octaves, arpeggios, or sirens** from the
  shared root at one note per beat using the selected tempo, moving by a semitone
  between passes across an octave. A **3–2–1 countdown** starts the first phrase;
  later passes begin immediately. Changing the root, scale, or pattern stops the
  current warm-up.
- **Pitch Echo** lets you select a Warm-Up pattern. Every sequence begins with a
  **3–2–1 countdown**, plays at the selected tempo, then shows another **3–2–1 pause**
  before recording your response. Your match score stays visible until you choose
  **Next Sequence**; each next sequence transposes by a semitone.

Hold a note to see the pitch-width and vibrato/stability readout.

### Vocal range & tessitura

Detected pitches update the observed low and high notes. The tessitura estimate is
the central 80% of stable pitch samples accumulated across sessions; the blue graph
band marks this estimate when available. It is recalculated from all saved samples
and can move higher or lower as your singing changes. A pitch counts as stable when
its width over the recent 900 ms is at most 0.25 semitones. The estimate describes
where your pitch has been most stable, not a direct measurement of which notes feel
comfortable or natural. Global saved values are always shown; the session range
starts with a prompt and appears after the app records a pitched note. Use
**Settings → Clear Range & Tessitura History** to erase saved history and current
session values.
History is stored locally in your user's application data directory and carries over
between sessions.

Range tracking also filters out quiet or brief pitch detections. In **Settings →
Range Sensitivity**, lower values require a louder signal; detected pitches must
remain for at least 400 ms before they affect the range. The default is **35%**.
Changing this setting affects new samples, not range history already saved; clear
history if you want to discard earlier false detections.

### Settings & appearance

The appearance follows the operating system theme by default. Choose **Settings →
Appearance → System, Light, or Dark** to select a mode.
**Settings → Range Sensitivity** opens a slider submenu for the range noise gate;
higher values accept quieter signals. **Settings → Highlight Opacity** opens a
separate submenu for adjusting the key/scale highlight strength.
**Help → About PitchHelper** summarizes the controls and note colors.

## Defaults & reference

| Setting                        | Default                     | Notes                                                    |
| ------------------------------ | --------------------------- | -------------------------------------------------------- |
| In-tune tolerance              | 15 cents                    | Green band; outside this is red (flat) or blue (sharp).  |
| Track volume                   | 50%                         | Playback volume, settable in the slider.                 |
| Tempo                          | 100 BPM                     | Adjustable from 40 to 240 BPM.                           |
| Pitch sensitivity              | 80%                         | Higher sensitivity lowers the detection threshold.        |
| Range sensitivity              | 35%                         | Lower values require louder, sustained pitches.           |
| Scale-row highlight opacity    | 4%                          | In-scale tint; chord tones are stronger.                 |
| Reference tone                 | Root or fifth               | Shown as a yellow target line.                           |
| Pitch detection range          | 25 – 2,500 Hz               | Autocorrelation over the mic buffer.                     |
| Graph bounds                   | C2–C6                       | Adjustable from C0 to C9.                                |
| Graph zoom                     | 1× (half the range)         | 0.5× shows all LOW/HIGH notes.                            |
| Metronome                      | Off                         | Enable with the Metronome controls.                      |
| Metronome volume               | 35%                         | Adjustable independently of track volume.               |
| Pitch Echo sequence            | Warm-Up pattern             | Score is shown until you advance to the next sequence.   |
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
| `assets/pitchhelper-icon.png` | Transparent application-window icon.                                      |

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

Build a native release binary on the target platform:

```sh
cargo build --release
```

Build separately for Windows, Linux, and macOS. On Linux, install the ALSA and
desktop development libraries before building; on Debian-based distributions:

```sh
sudo apt-get install pkg-config libasound2-dev libx11-dev libxkbcommon-dev \
  libwayland-dev libxrandr-dev libxinerama-dev libxcursor-dev libxi-dev \
  libxkbcommon-x11-dev libgl1-mesa-dev
```

The resulting binary is in `target/release/`. GitHub Actions builds and tests the
supported platforms; pushing a `v*` tag creates a GitHub release with packaged
Windows x86-64, Linux x86-64, macOS Intel, and macOS Apple Silicon binaries.

## Running the tests

```sh
cargo test
```

## Copyright & license

Copyright © 2026 Richard Fioravanti. *PitchHelper* is a working name for the project
and is not claimed as a registered trademark.

No open-source license is currently specified; the code is provided as-is. To request
a license or contribute, please get in touch.
