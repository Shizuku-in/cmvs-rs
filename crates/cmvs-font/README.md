# cmvs-font

Text rendering for CMVS hosts, replacing the original engine's GDI rasterizer with OpenType rendering through `ab_glyph`. Output is not claimed to be pixel-identical to GDI. Script layout stays in `cmvs-core`.

## Contents

- `font`: `FontRenderer` loads a font file or collection face, provides glyph metrics to the engine and renders message and object text.
- `font_library`: resolves the face names requested by scripts, standing in for GDI's `CreateFontA`. The explicit font serves the default face; other faces are found by family name in the directories from `cmvs_native::font_directories`. Fonts the user imports are kept in the user directory.
- `glyph_cache`: LRU of rasterized glyphs, keyed by face and every style that affects pixels. It keeps up to 512 styles and 9 MiB of RGBA pixels, so a typical full page remains cached across frames. The byte budget also accommodates the old cache's 128 maximum-size glyphs. Evicted pixels remain valid while draw objects reference them.

## Used by

`cmvs-desktop`, `cmvs-host`, and the Android, Web, Switch and PS Vita players.
