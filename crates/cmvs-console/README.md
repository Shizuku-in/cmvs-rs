# cmvs-console

Host logic shared by the Switch and PS Vita players. Each console describes its screen size, buttons and labels with a `Console`, and input arrives as `cmvs_native::PadInput` samples.

## Contents

| Module | Purpose |
|---|---|
| `controls` | Controller, stick and touch mapping to engine input codes |
| `library` | Game directories on the memory card; a game is only identified once it is picked |
| `menu` | Game selection screen, shown when more than one game is installed |
| `error_screen` | Full-screen error page shown before leaving a failed game |
| `canvas` | CPU-drawn host pages, laid out on a 720-unit grid and scaled to the screen |
| `encoding` | Game text encoding from `encoding.txt` in the game directory, or detected from startup text with bounded archive-script fallback |
| `text_input` | Save-comment dialog through the system keyboard, in the game text encoding |
| `gpu` | Executes the shared scene draw queue on a console GPU, with a content-keyed texture cache |

## Testing

The logic runs on a development machine:

```sh
cargo test -p cmvs-console
```

The ignored `menu_preview` and `error_preview` tests render the pages to PNG. They need `CMVS_MENU_FONT`, and setting `CMVS_MENU_SCREEN=vita` renders at 960×544.
