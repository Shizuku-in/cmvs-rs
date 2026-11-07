# cmvs-native

Operating-system services for CMVS hosts behind one cross-platform API. It has no VM or game-command logic. Every platform module implements the same functions, and a service a platform does not have returns `NativeError::Unsupported`.

## Services

| Platform | Provides |
|---|---|
| Windows | Font picker, save-comment text dialog, system font directories, masked cursors |
| Linux | Font picker and text dialog, Fontconfig font directories |
| macOS | Font picker and text dialog, system font directories |
| Android | Native window handle for GPU surfaces, font picker |
| Switch | Window and GPU device, controller and touch input, audio output, software keyboard, system font, CPU boost |
| PS Vita | Window and GPU, buttons and touch, audio output, IME and message dialogs, clock requests |

`PadInput` and the `GpuDevice` / `GpuTexture` types are shared by the console players through `cmvs-console`.
