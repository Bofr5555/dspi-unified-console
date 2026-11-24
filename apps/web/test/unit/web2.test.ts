// WP WEB2 unit tests: the channel colours as Qt's ChannelPalette (REVD W2), the matrix's "+" and
// "−" keys (REVD G1: they type a gain, a modifier steps it) and a slider's filled part (§11).

import { test } from "node:test";
import assert from "node:assert/strict";
import { channelColour, INPUT_COLOURS, inputColour, OUTPUT_COLOURS, outputColour, PDM_COLOUR } from "../../src/screens/eq/model";
import { stepSign } from "../../src/screens/matrix/model";
import { rangeFill } from "../../src/ui/design";

test("WEB2 W2: a channel's colour is Qt's ChannelPalette's, by its input or output number", () => {
  const rp2350 = { numInputChannels: 2, numOutputChannels: 9 };
  // ChannelPalette.qml: inputs blue, red …; outputs teal, green …; the PDM sub violet
  assert.equal(inputColour(0), "#4a8fe3");
  assert.equal(inputColour(1), "#f57373");
  assert.equal(channelColour(rp2350, 0), INPUT_COLOURS[0]);
  assert.equal(channelColour(rp2350, 1), INPUT_COLOURS[1]);
  assert.equal(channelColour(rp2350, 2), "#45c2a3"); // Out 1
  assert.equal(channelColour(rp2350, 3), "#59d180"); // Out 2: green, not a grey that reads as off
  assert.equal(channelColour(rp2350, 2 + 7), OUTPUT_COLOURS[7]);
  assert.equal(channelColour(rp2350, 2 + 8), PDM_COLOUR);
  // The same output on a device with eight inputs keeps its colour
  assert.equal(channelColour({ numInputChannels: 8, numOutputChannels: 9 }, 8), OUTPUT_COLOURS[0]);
  // RP2040: four outputs and the PDM sub as the fifth
  assert.equal(outputColour({ numOutputChannels: 5 }, 4), PDM_COLOUR);
  assert.equal(outputColour({ numOutputChannels: 5 }, 3), OUTPUT_COLOURS[3]);
  // Past the palette: the accent, as Qt
  assert.equal(inputColour(8), "#2563eb");
  assert.equal(outputColour({ numOutputChannels: 16 }, 12), "#2563eb");
});

test("WEB2 G1: + and − are read by the key's character or place; other keys are not steps", () => {
  assert.equal(stepSign({ key: "+" }), 1);
  assert.equal(stepSign({ key: "=" }), 1);
  assert.equal(stepSign({ key: "+", code: "NumpadAdd" }), 1);
  assert.equal(stepSign({ key: "-" }), -1);
  assert.equal(stepSign({ key: "_" }), -1);
  assert.equal(stepSign({ key: "−" }), -1);
  // Alt on a Mac turns "-" into a dash and "=" into "≠": the key's place still steps
  assert.equal(stepSign({ key: "–", code: "Minus" }), -1);
  assert.equal(stepSign({ key: "≠", code: "Equal" }), 1);
  assert.equal(stepSign({ key: "6", code: "Digit6" }), 0);
  assert.equal(stepSign({ key: "Enter" }), 0);
});

test("WEB2 §11: a slider's filled part, clamped to its range", () => {
  assert.deepEqual(rangeFill(0, -60, 0), { "--fill": "100.00%" });
  assert.deepEqual(rangeFill(-30, -60, 0), { "--fill": "50.00%" });
  assert.deepEqual(rangeFill(-90, -60, 0), { "--fill": "0.00%" });
  assert.deepEqual(rangeFill(5, 0, 0), { "--fill": "0.00%" });
  assert.deepEqual(rangeFill(Number.NaN, 0, 1), { "--fill": "0.00%" });
});

test("WEB2 W11: a page that needs its width keeps it whole; the inspector goes under", async () => {
  const { BESIDE_MIN_PX, INSPECTOR_PX, placement } = await import("../../src/ui/inspector/model");
  // The matrix's grid at nine outputs: 168 + 12 + 9 × 56 + 90 = 774 px
  assert.equal(placement(1290, false, 774), "beside");
  assert.equal(placement(774 + INSPECTOR_PX - 1, false, 774), "under");
  assert.equal(placement(774 + INSPECTOR_PX, false, 774), "beside");
  // A page that asks for nothing: as before
  assert.equal(placement(BESIDE_MIN_PX, false), "beside");
  assert.equal(placement(BESIDE_MIN_PX - 1, false, 0), "under");
  assert.equal(placement(2000, true, 774), "hidden");
});
