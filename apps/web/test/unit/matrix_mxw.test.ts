// WP WEB / MXW unit tests: typed numbers as Qt's fields read them (USR1: a decimal comma, U+2212,
// a unit after the number) through every field's parser, and the matrix's rules without I/O
// (the tile's and the row tail's words, the summary, the keys' moves, the gain's steps, Link L/R
// pairs: the mirrored crosspoint and the per-device choice).

import { test } from "node:test";
import assert from "node:assert/strict";
import { plainNumber, readNumber } from "../../src/ui/number";
import { parseFreq, parseNumber } from "../../src/screens/eq/model";
import { parseDistance } from "../../src/screens/eq/align";
import { parseLevel } from "../../src/screens/outputs/model";
import { parseValue } from "../../src/screens/effects/model";
import { limitValue } from "../../src/screens/installer/model";
import { feedsOf, linkPairsOf, mirrorOf, moveCell, runs, setLinkPairs, sourcesOf, stepGain, summaryText, tailText, tileGain } from "../../src/screens/matrix/model";

test("WEB numbers: a decimal comma, U+2212, spaces and a trailing unit, as Qt's BorderedField.number()", () => {
  assert.equal(readNumber("3,5"), 3.5);
  assert.equal(readNumber("−2,5"), -2.5);
  assert.equal(readNumber("− 12"), -12);
  assert.equal(readNumber("+1.5 dB", ["dB"]), 1.5);
  assert.equal(readNumber("-6dbfs", ["dBFS", "dB"]), -6);
  assert.equal(readNumber("-6 dB", ["dBFS", "dB"]), -6);
  assert.equal(readNumber("1 000"), 1000);
  assert.equal(readNumber(".5"), 0.5);
  for (const bad of ["", "abc", "1,5.0", "1,2,3", "1.2.3", "--1", "5 Hz"]) assert.equal(readNumber(bad), null, bad);
  assert.equal(plainNumber("−2,5 ms", ["ms"]), "-2.5");
});

test("WEB numbers: every field's parser takes them", () => {
  assert.equal(parseLevel("−2,5 dB", "db"), -2.5);
  assert.equal(parseLevel("4,125 ms", "ms"), 4.125);
  assert.equal(parseLevel("-6 dB", "dbfs"), -6);
  assert.equal(parseNumber("−3,5 dB", "db"), -3.5);
  assert.equal(parseNumber("0,707", ""), 0.707);
  assert.equal(parseFreq("2,5 kHz"), 2500);
  assert.equal(parseFreq("1 000 Hz"), 1000);
  assert.equal(parseFreq("A4"), 440);
  assert.equal(parseValue("−1,5 dB", { unit: "dB" }), -1.5);
  assert.equal(parseValue("650,5 Hz", { unit: "Hz" }), 650.5);
  assert.equal(parseValue("85 dB SPL", { unit: "dB SPL" }), 85);
  assert.equal(parseDistance("1,5 m"), 1.5);
  assert.equal(limitValue("−2,5 dB", { min: -20, max: 20, unit: "dB" }), -2.5);
  assert.equal(limitValue("", { min: -20, max: 20, unit: "dB" }), undefined);
});

test("MXW tile and tail: the gain without its unit; the outputs a row feeds", () => {
  assert.equal(tileGain(0), "0");
  assert.equal(tileGain(-0.001), "0");
  assert.equal(tileGain(-6), "-6");
  assert.equal(tileGain(1.5), "+1.5");
  assert.equal(tileGain(-12.25), "-12.25");
  assert.equal(tailText([0, 1, 2]), "→ Out 1, 2, 3");
  assert.equal(tailText([]), "→ nothing");
  assert.equal(runs([3, 5, 6, 7, 8]), "4, 6–9");
  assert.equal(runs([0, 1]), "1, 2");
  assert.equal(runs([]), "");
});

test("MXW routes: feeds, sources and the inspector's summary", () => {
  const routing = [
    [true, false, false, false],
    [false, true, false, false],
    [true, true, false, false],
  ];
  assert.deepEqual(feedsOf(routing, 2, 4), [0, 1]);
  assert.deepEqual(sourcesOf(routing, 0, 3), [0, 2]);
  assert.deepEqual(sourcesOf(routing, 0, 2), [0], "only the shown inputs");
  assert.equal(summaryText(routing, [true, true, false, true], 3, 4), "4 crosspoints · 3 of 4 outputs on · Out 3, 4 fed by nothing");
  assert.equal(summaryText([[true]], [true], 1, 1), "1 crosspoint · 1 of 1 outputs on");
});

test("MXW Link L/R pairs: the partner row one column over, inside the output pair; none without a partner", () => {
  assert.deepEqual(mirrorOf(0, 0, 2, 9), { i: 1, o: 1 }, "L→Out 1 mirrors R→Out 2");
  assert.deepEqual(mirrorOf(1, 1, 2, 9), { i: 0, o: 0 });
  assert.deepEqual(mirrorOf(0, 1, 2, 9), { i: 1, o: 0 }, "L→Out 2 mirrors R→Out 1");
  assert.deepEqual(mirrorOf(4, 6, 8, 9), { i: 5, o: 7 });
  assert.equal(mirrorOf(0, 8, 2, 9), null, "the PDM output has no partner");
  assert.equal(mirrorOf(2, 0, 3, 9), null, "an odd count's last input has none");
  const kept = new Map<string, string>();
  const store = { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
  assert.equal(linkPairsOf(store, "A"), false, "off by default");
  setLinkPairs(store, "A", true);
  assert.equal(linkPairsOf(store, "A"), true);
  assert.equal(linkPairsOf(store, "B"), false, "per device");
  setLinkPairs(store, "A", false);
  assert.equal(linkPairsOf(store, "A"), false);
  assert.equal(linkPairsOf(null, "A"), false);
  const broken = {
    getItem: () => {
      throw new Error("denied");
    },
    setItem: () => {
      throw new Error("denied");
    },
  };
  assert.equal(linkPairsOf(broken, "A"), false);
  setLinkPairs(broken, "A", true);
});

test("MXW keys: arrows, Home/End, PgUp/PgDn within the grid; the gain's steps", () => {
  const at = { i: 1, o: 3 };
  assert.deepEqual(moveCell(at, "ArrowLeft", 4, 9), { i: 1, o: 2 });
  assert.deepEqual(moveCell(at, "ArrowRight", 4, 9), { i: 1, o: 4 });
  assert.deepEqual(moveCell(at, "ArrowUp", 4, 9), { i: 0, o: 3 });
  assert.deepEqual(moveCell(at, "ArrowDown", 4, 9), { i: 2, o: 3 });
  assert.deepEqual(moveCell({ i: 0, o: 0 }, "ArrowUp", 4, 9), { i: 0, o: 0 }, "kept inside");
  assert.deepEqual(moveCell({ i: 3, o: 8 }, "ArrowRight", 4, 9), { i: 3, o: 8 });
  assert.deepEqual(moveCell(at, "Home", 4, 9), { i: 1, o: 0 });
  assert.deepEqual(moveCell(at, "End", 4, 9), { i: 1, o: 8 });
  assert.deepEqual(moveCell(at, "PageUp", 4, 9), { i: 0, o: 3 });
  assert.deepEqual(moveCell(at, "PageDown", 4, 9), { i: 3, o: 3 });
  assert.equal(moveCell(at, "x", 4, 9), null);
  const r: [number, number] = [-60, 12];
  assert.equal(stepGain(0, 1, false, r), 0.5);
  assert.equal(stepGain(0, -1, true, r), -3);
  assert.equal(stepGain(-2.3, 1, false, r), -2);
  assert.equal(stepGain(11, 1, true, r), 12, "within the range");
});
