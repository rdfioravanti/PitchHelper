# PitchHelper

PitchHelper is a native Rust desktop app for singers. It listens to a selected
microphone, shows the closest note and tuning deviation, and plots pitch across
a moving four-bar 4/4 timeline.

## Run

Install the Rust toolchain, then run:

```sh
cargo run
```

Use **Start Listening** to begin pitch tracking and select an input device from
the adjacent menu. The list refreshes automatically when audio devices change.
Use the **Refresh Microphones** button to rescan immediately. **Pitch Sensitivity**
slider beneath the microphone controls lowers the detection
threshold for quieter singing; the meter shows level and warns when the signal
clips.
Notes within 15 cents of pitch are green, flat notes are red, and sharp notes
are blue. Enharmonic spellings are shown with ASCII sharps and flats.

Use **File > Open audio** to load WAV, MP3, FLAC, or OGG backing tracks. PitchHelper
opens the file picker and analyzes the track for a steady tempo in the background,
so the interface remains responsive while the picker is open and while the track
loads. It applies the detected tempo to the timeline; use the tempo slider to
override it. Play, pause, or stop the track from the playback controls, and adjust
its volume with the track volume slider (default 50%). Enable the metronome and
adjust its volume with the **Metronome** controls on the main screen.
With a track loaded, the playhead and metronome follow playback: they pause with
the track and **Stop** resets the playhead to the beginning. **Clear Track**
returns the playhead to free-running mode.
The contrasting amber waveform above the pitch graph shows the track's relative
volume. Hover to preview a timestamp, then click or drag to seek; the current and
total track times appear beside the name. A/B handles default to the full track;
drag them to loop a section, or right-click the waveform to reset either handle
or both. Seeking outside the visible four-measure cycle clears the pitch trace.

Use **Zoom**, **LOW**, and **HIGH** beside the tempo control to configure the
graph. LOW and HIGH set the graph's boundaries. The default 1x view zooms into
those bounds; 0.5x shows the full range. In free singing mode, the graph follows
live pitch smoothly when it nears the edge, without moving beyond LOW/HIGH.
During warm-ups and pitch echo, it centers on the exercise's target notes. The
selectable range includes C1 and extends above C6. Note names use ASCII
accidentals (for example, C# and Db).

Choose a **Key** and **Scale** in the Practice controls to highlight in-scale
note rows; chord tones are highlighted more strongly. Adjust their subtle
4% default opacity in **Settings**. The **Reference Tone** control plays the
selected key's root or fifth, shown as a yellow target line. The key's octave
is selected alongside its root and is shared by the reference tone, warm-ups,
and pitch echo. **Warm-Up** plays 5-tone scales, octaves, arpeggios, or sirens
from that shared root (C1–C6), and moves by a semitone after each pass. Changing
the root, octave, scale, or pattern stops the current warm-up. **Pitch Echo**
shows a 3–2–1 countdown in the graph, plays a three-note phrase, then scores
the sung response. Hold a note to see the pitch-width and vibrato/stability
readout. Stable notes contribute to the vocal range and tessitura display;
that history is saved locally in the user's application data directory.

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
