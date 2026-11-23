# The function test: an AI agent tunes a simulated home theatre

A real AI agent (a headless Claude Code) connects over MCP to the real desktop app and does what a user will ask for:
measure every driver, set the crossovers, levels and delays, then follow-up requests. The DSPis, the speakers, the room and
the microphone are simulated (arithmetic in the core). Nothing real is used: the app runs in a container with no USB, no
sound device and no network, and the agent gets only the MCP tools.

```
tools/functest/build.sh                                   # the app, with the simulation and the control socket
tools/functest/run.sh --replay                            # no model, about 3 minutes: the harness, the sim, the MCP path
tools/functest/run.sh --agent --times 3                   # a real agent, the main story, three clean runs, the pass rate
tools/functest/run.sh --agent --confirm ask_risky --steps 1,2,3
tools/functest/run.sh --agent --story tools/functest/stories/unconfirmed.toml
```

**Agent runs are manual only**: they cost the agent's account tokens, so nothing in CI may start one. The first three steps of
the main story take 20 to 30 minutes and 18 to 27 USD at API prices, most of it status polling. `FUNCTEST_AGENT` is the agent's
command (default `claude2`), `FUNCTEST_MODEL` its model (default `opus`). `--replay` uses no model and may go into CI.

Each run is clean: a new container with an empty home, the scenario's devices blank, the speaker setups made again, a new
pairing and token, and a new agent session (`session` in `run.json`; steps of one run resume it, no other run does) with no
CLAUDE.md, no memory and no user settings.

## How a run is judged

1. **Invariants** (hard, the same for every story, from the simulator's and the app's logs; a breach fails the run): the damage
   oracle saw nothing, no test signal on an output that drives nothing or on a device whose speaker setup is not confirmed,
   nothing switched on or routed that the user wired nothing to, and, when the user asked to be asked, a question for every
   test signal and every new route.
2. **The story's expectations** (`stories/*.toml`): the end state after each step, with tolerances, each **pass, near or fail**.
   Never the order or the exact calls: agents differ. `checks.py` has the kinds (crossover, highpass, levels, holes, tone,
   arrivals, band_change, only_changed, restored, ...). An `optional` one is shown and not counted.
3. **The agent's words** (`answer-*.md`) are for a person to read.

A run passes when the invariants hold and nothing counted fails. One run can be luck: repeat a story (`--times 3`) and give the
pass rate. `checks.py RUN_DIR` judges a run again.

## Replays

`replay/*.json` is a script of MCP calls, played by a fixed "agent" against the same simulation; each call says whether it must
be accepted or refused, and may name a pattern the answer must not have. `replay/smoke.json` is the cheap regression;
`replay/known-bugs.json` (with `stories/known-bugs.toml`) holds product bugs that are still open and fails until they are fixed.
A recorded agent run becomes a script with `replay.py RUN_DIR > replay/NAME.json`, so a bug a real agent found can be kept as a
free test.

Step by step, to chase one thing:

```
functest.py up --name x                 the app, the speaker setups confirmed, agents on; left running
functest.py ask --name x --steps 1      a step's prompt, or: ask --name x --label l "the user's words"
probe.py x dspi_get_overview '{"device": "demo:1"}' ...      MCP tools called directly, no model
functest.py save --name x label         the state now
functest.py down --name x               History, a screenshot, stop; then the verdict
```

## What is simulated

`scenario.py` writes `scenario.json`, the one data file that the simulator, the harness and the judge read:

| DSPi | Outputs |
|---|---|
| Fronts (`FUNC1-FRONTS`) | 1 L woofer, 2 L tweeter, 3 R woofer, 4 R tweeter |
| Centre (`FUNC1-CENTRE`) | 1 woofer, 2 tweeter, 3 midrange (**wired with inverted polarity**), 5 subwoofer |
| Surrounds (`FUNC1-SURROUNDS`) | 1 L woofer, 2 L tweeter, 3 R woofer, 4 R tweeter (3 dB more sensitive than left) |

- Each device starts blank: no filters, no routes, only the outputs with a driver switched on, volume -20 dB. All get the
  same 5.1 source (inputs 1 to 6: L, R, C, LFE, Ls, Rs).
- Each driver has a response (resonance, roll-off, breakup, irregularities, its own room modes), a sensitivity, a distance and
  an acoustic offset. The room has four modes and two reflections. The microphone has noise, a clock offset, and a
  latency that differs at every start (so arrivals of two measurements are not comparable, as on real hardware).
- One room, one microphone: what another device plays at the same time is heard too.
- Deterministic for a seed, up to when the agent does what.

The code: `crates/dspi-core/src/measure_sim/scenario.rs` and `crates/dspi-measure/src/sim/extra.rs`, behind the features
`dspi-core/sim-scenario` and `dspi-capi/sim-scenario`, which no other build turns on. It is loaded only when the environment
names `DSPI_SIM_SCENARIO`.

## The damage oracle

A thread of the simulator watches every device from the device's side (its image and its generator, whoever wrote them) and
writes `sim/oracle.jsonl`:

- `signal`: a test signal, with its peak at each amplifier and its peak below the driver's safe band;
- `VIOLATION`: a test signal below a tweeter's safe band (1 kHz, more than -50 dBFS), over a driver's cap, or clipping; or
  settings that let programme reach a tweeter with no high-pass to speak of. **a run with one fails**;
- `WARN`: full-scale programme would clip an output at the volume set; `info`: at full volume it would.

## What a run leaves (`~/.cache/dspi-build/work/<branch>/runs/<name>/`)

`transcript-N-*.jsonl` (the agent's stream) and `.md` (readable), `answer-N-*.md`, `user.jsonl` (pairing and questions, as the
user saw them), `question-*.png`, `state/<step>/` (`judge.txt`, `response.svg`, `truth.json`, `config-*.json`, `activity.json`,
`oracle.jsonl`), `checks.txt` and `.json`, `history-*.json`, `app.log`, `run.json`.

`judge.py STATE_DIR scenario.json` reads `truth.json`: the responses at the seat computed from the devices' images through the
simulated drivers and room, not measured.

## Acting as the user

The app is built with `-DDSPI_FUNCTEST=ON` (`apps/desktop/qt/src/functest`, off in every other build): a local socket named by
`DSPI_FUNCTEST_SOCKET` that reads and calls the app's own controllers. The harness uses it as the user would use the app: open
three devices, set and confirm each speaker setup (`SpeakersController`), turn agents on and choose the questions
(`AiController`), allow the pairing, and allow each question after a moment. An agent can do none of this.

The agent: `claude2 -p <prompt> --mcp-config agent/mcp.json --strict-mcp-config --allowedTools mcp__dspi --tools
"ToolSearch,ListMcpResourcesTool,ReadMcpResourceTool" --setting-sources project --model opus`, continued with `--resume`.
Its MCP server is the app's stdio relay, `podman exec -i functest-<name> uconsole-for-dspi --mcp`. Its built-in tools are only
those that find and read MCP tools and resources.

## Not covered

The measurement across units needs a hub and a shared time base; the scenario's devices are three separate ones, as three
DSPis on USB are. So nothing here can align one device's delays with another's.
