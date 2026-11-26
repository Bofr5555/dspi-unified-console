# The front page's screenshots

Real renders of the apps on demo devices, no hardware, made in containers by
[`tools/shots/readme.sh`](../../tools/shots/readme.sh): the desktop app's Modern layout on the function test's simulated
home theatre (`tools/functest`, the story `show-gallery.toml`: three DSPi units, a simulated microphone), the web
console on the core's demo DSPi and the demo hub (`uconsole --demo hub --alink`), the Terminal on the demo DSPi:

```sh
tools/shots/readme.sh ~/.cache/dspi-build/work/shots       # render, then crop into docs/images/
SKIP_RENDER=1 tools/shots/readme.sh ~/.cache/dspi-build/work/shots   # only the crops
```

It starts the function test's app on a 1600×1000 screen (`functest.py up`), takes its pages with `gallery.py` and a
measurement by the user with `shots2.py`, takes the web console's two pictures (`tools/shots/web.sh` with
`apps/web/test/shots/readme_web.spec.ts`) and the Terminal's (`tools/shots/terminal.sh`), then crops, scales to at most 1400 px wide and recompresses each picture with ImageMagick
in the build image. The light theme throughout, but the Terminal, which is shown dark.

| File | What |
|---|---|
| `qt-main.png` | The desktop app, Modern layout: Equaliser › PEQ with an output's bands, three units open as tabs |
| `web-eq.png` | The web console on the demo DSPi: the Equaliser |
| `web-system.png` | The web console on the demo hub: the System view of an alink installation |
| `qt-overview.png` | The desktop app, Modern layout: the Overview of a three-way speaker and a sub |
| `qt-measure.png` | The desktop app, Modern layout: Measure › Results, a result against its target (the simulated microphone) |
| `terminal.png` | The `uconsole` Terminal's interface on the demo DSPi |

Keep each picture well under 400 KB. A build cache, never `/tmp`, holds the renders.
