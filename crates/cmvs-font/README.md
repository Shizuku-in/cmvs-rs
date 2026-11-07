# cmvs-font

Text rendering for CMVS hosts, replacing the original engine's GDI rasterizer with OpenType rendering through `ab_glyph`. Output is not claimed to be pixel-identical to GDI. Script layout stays in `cmvs-core`.

## Contents

- `font`: `FontRenderer` loads a font file or collection face, provides glyph metrics to the engine and renders message and object text.
- `font_library`: resolves the face names requested by scripts, standing in for GDI's `CreateFontA`. The explicit font serves the default face; other faces are found by family name in the directories from `cmvs_native::font_directories`. Fonts the user imports are kept in the user directory.
- `glyph_cache`: bounded LRU of rasterized glyphs, keyed by face and every style that affects pixels.

## Used by

`cmvs-desktop`, `cmvs-host`, and the Android, Web, Switch and PS Vita players.
