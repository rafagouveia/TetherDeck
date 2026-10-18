# Visual examples

These are real captures of TetherDeck on Windows and an iPhone running Calculator.
The workspace screenshots are available in English, Portuguese and Spanish.

| Asset | Content |
| --- | --- |
| `workspace.jpg`, `workspace.pt-BR.jpg`, `workspace.es.jpg` | Connected workspace |
| `remote-screen.jpg` | Independent remote screen and controls |
| `elements.jpg` | WDA inspection, filtering and selection of a native Calculator key |
| `settings.jpg`, `settings-light.jpg` | Language and appearance settings |
| `workflow.gif` | Sidebar, settings, themes and language navigation; 1000 × 656, about 10 seconds |
| `remote-control.gif` | Actual remote touches producing 2 + 2 = 4; 420 × 780, about 5 seconds |

## Capture and privacy

`TETHERDECK_PRESENTATION=1` enables a process-only presentation preference. It
replaces device names and known connection identifiers in supported application
views without changing the device, session or API data. It is **not general
redaction**: device pixels, element labels, logs, integration panels and signing
details can still contain real information. Use a benign app and review every
capture before publishing. These examples contain no personal signing material.

JPEG files preserve the native window captures. GIF playback uses sampled frames
at 8 frames per second, with shortened pauses and short holds. These animations
demonstrate a workflow; their playback speed is not a transmission FPS or
end-to-end latency measurement. Static screenshots remain available when the
optional animation sections in the README are closed.

## Rebuild the animations

Prepare reviewed JPEG sequences named `workflow-0000.jpg`, `workflow-0001.jpg`,
and so on, plus `remote-0000.jpg`, `remote-0001.jpg`, and so on. Then run:

```powershell
.\scripts\export-demo-media.ps1 -FramesDirectory <reviewed-frame-directory> -Ffmpeg <ffmpeg.exe>
```

The helper writes only the two GIFs to this directory by default. Original
recording frames and local session scripts are excluded from the repository.
