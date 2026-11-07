# cmvs-input

Touch input for touch-screen hosts, with no window-system dependencies.

## API

- `Viewport` maps between physical surface pixels and script coordinates for the letterboxed game rectangle, honouring the display mode.
- `TouchInput` turns `TouchEvent`s into engine input. Hosts sample it once per engine frame, so a short tap still reaches the next frame. It cancels input when the host loses focus.

Gesture recognition and `key.cfg` action bindings stay in `cmvs-core`.

## Used by

`cmvs-desktop` and the Android player.

## Testing

```sh
cargo test -p cmvs-input
```
