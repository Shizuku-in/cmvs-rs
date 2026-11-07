# cmvs-audio

Audio for CMVS hosts: streaming Ogg Vorbis (`lewton`) and PCM WAVE decoders, an engine mixer and native device output (`rodio`).

## Features

| Feature | Adds |
|---|---|
| none | `vorbis` and `pcm_wave` decoders only |
| `mixer` | Engine channel state, envelopes and software mixing. The host pulls PCM through `PcmOutput::fill_i16`, which is how the Switch and PS Vita players feed their audio ports |
| `native` (default) | `mixer` output through the default system device (`NativeAudio`) |

`HostAudioControl` lets a host mute output while it is backgrounded. BGM position and gain are exposed to the engine for its own timing.

## Used by

`cmvs-desktop` (`audio-output` feature), `cmvs-host`, and the Android, Web, Switch and PS Vita players.
