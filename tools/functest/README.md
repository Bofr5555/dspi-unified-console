# The function test: an AI agent tunes a simulated home theatre

A real AI agent (a headless Claude Code) connects over MCP to the real desktop app and does what a user will ask for:
measure every driver, set the crossovers, levels and delays, then follow-up requests. The DSPis, the speakers, the room and
the microphone are simulated (arithmetic in the core). Nothing real is used: the app runs in a container with no USB, no
sound device and no network, and the agent gets only the MCP tools.

```
tools/functest/build.sh                                        # the app, with the simulation and the control socket
python3 tools/functest/functest.py run --name r1               # never-ask: every prompt of prompts.md, one after the other
python3 tools/functest/functest.py run --name r2 --confirm ask_risky --steps 1,2,3
python3 tools/functest/functest.py run --name r3 --unconfirmed Centre --prompts tools/functest/prompts-unconfirmed.md
```

A whole `run` takes about 30 minutes; three steps of the main story cost 18 to 27 USD at API prices,
most of it status polling. `FUNCTEST_AGENT` is the agent's command (default `claude2`), `FUNCTEST_MODEL` its
model (default `opus`).
The exit code is 1 when the damage oracle saw a violation or the agent did not finish.

Step by step, to chase one thing:

```
functest.py up --name x                 the app, the speaker setups confirmed, agents on; left running
functest.py ask --name x --steps 1      a prompt of prompts.md, or: ask --name x --label l "the user's words"
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
  settings that let programme reach a tweeter with no high-pass to speak of. **A run with one fails**;
- `WARN`: full-scale programme would clip an output at the volume set; `info`: at full volume it would.

## What a run leaves (`~/.cache/dspi-build/work/<branch>/runs/<name>/`)

`transcript-N-*.jsonl` (the agent's stream) and `.md` (readable), `answer-N-*.md`, `user.jsonl` (pairing and questions, as the
user saw them), `question-*.png`, `state/<step>/` (`judge.txt`, `response.svg`, `truth.json`, `config-*.json`, `activity.json`,
`oracle.jsonl`), `history-*.json`, `app.log`, `run.json`.

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
