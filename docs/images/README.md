# The front page's screenshots

Real renders of the apps on their demo devices and the demo hub (`uconsole --demo hub --alink`), no hardware,
made in containers by [`tools/shots/readme.sh`](../../tools/shots/readme.sh):

```sh
tools/shots/readme.sh ~/.cache/dspi-build/work/shots       # render, then crop into docs/images/
SKIP_RENDER=1 tools/shots/readme.sh ~/.cache/dspi-build/work/shots   # only the crops
```

It runs the Qt smoke driver's screenshot sections (`ci/check.sh --step qt-offscreen` with `DSPI_SMOKE_SECTION`),
the web console's tour (`tools/shots/web.sh`) and the Terminal's (`tools/shots/terminal.sh`), then crops, scales to
at most 1400 px wide and recompresses each picture with ImageMagick in the build image. The light theme throughout,
but the Terminal, which is shown dark.

| File | What |
|---|---|
| `qt-main.png` | The Qt desktop app's main window on the demo DSPi |
| `web-simple.png` | The web console in Simple mode: the front page |
| `web-system.png` | The web console on the demo hub: the System view of an alink installation |
| `qt-system.png` | The Qt app's System window on the demo hub |
| `qt-measure.png` | The Qt app's Measure window: a result against its target (the demo's simulated microphone) |
| `terminal.png` | The `uconsole` Terminal's interface on the demo DSPi |

Keep each picture well under 400 KB. A build cache, never `/tmp`, holds the renders.
