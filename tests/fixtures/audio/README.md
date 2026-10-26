# Synthetic Vorbis test fixtures

These files contain generated sine waves only, with no game or recorded content.
Generated with local FFmpeg 2024-03-14 (libvorbis encoder):

```text
ffmpeg -f lavfi -i sine=frequency=440:sample_rate=8000:duration=0.08 -c:a libvorbis -q:a 3 -fflags +bitexact -flags:a +bitexact mono440.ogg
ffmpeg -f lavfi -i sine=frequency=880:sample_rate=8000:duration=0.08 -c:a libvorbis -q:a 3 -fflags +bitexact -flags:a +bitexact mono880.ogg
```

Both decode to 768 mono samples at 8000 Hz with lewton 0.10.2 and the local FFmpeg Vorbis decoder.
The page granule declares 640 samples; the decoder output includes the initial block padding.
Tests use the observed decoder output, not the requested sine duration.
The generated tones carry no third-party expressive content; no FFmpeg source code is included.
