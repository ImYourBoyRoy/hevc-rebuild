// ./desktop/tests/explanations.test.mjs
// Plain-language help for encoder settings and the advisories shown beside them. Run: pnpm test:explanations

import assert from "node:assert/strict";
import test from "node:test";
import { crfBand, encoderAdvisories, presetHelp, tuneHelp, CRF_DEFAULT } from "../src/lib/explanations.ts";

test("every CRF from 0 to 51 maps to exactly one named band, and the default is the recommended one", () => {
  const labels = new Set();
  for (let crf = 0; crf <= 51; crf += 1) {
    const band = crfBand(crf);
    assert.ok(band.label && band.summary, `CRF ${crf} has a band`);
    labels.add(band.label);
  }
  assert.ok(labels.size >= 5, "several distinct bands, not one blanket message");
  assert.equal(CRF_DEFAULT, 21);
  assert.equal(crfBand(CRF_DEFAULT).recommended, true);
  assert.equal(crfBand(40).recommended, false);
  assert.equal(crfBand(-5).label, crfBand(0).label, "out-of-range values clamp");
  assert.equal(crfBand(99).label, crfBand(51).label);
});

test("each tune has a name, a one-line summary and a plain explanation that does not assume jargon", () => {
  for (const tune of ["auto", "animation", "grain", "none"]) {
    const help = tuneHelp(tune);
    assert.ok(help.title && help.summary && help.detail, tune);
  }
  const grain = tuneHelp("grain");
  assert.match(grain.detail, /grain is the fine, random speckle/i);
  assert.match(grain.detail, /larger/i);
  assert.match(tuneHelp("auto").detail, /never turns on film grain/i);
  assert.equal(tuneHelp("bogus").title, tuneHelp("auto").title, "unknown values read as automatic");
});

test("presets explain the speed versus size trade and flag the recommended one", () => {
  assert.match(presetHelp("slow"), /recommended/i);
  assert.match(presetHelp("ultrafast"), /larger/i);
  assert.match(presetHelp("veryslow"), /slower/i);
  assert.ok(presetHelp("unknown-preset").length > 0);
});

test("film grain warns that dark and grainy films will stay large, at any CRF", () => {
  const [advisory] = encoderAdvisories({ tune: "grain", crf: 21 });
  assert.equal(advisory.level, "warning");
  assert.match(advisory.text, /dark/i);
  assert.match(advisory.text, /larger/i);
  assert.equal(encoderAdvisories({ tune: "auto", crf: 21 }).length, 0);
  assert.equal(encoderAdvisories({ tune: "none", crf: 24 }).length, 0);
});

test("extreme CRF values get an advisory in both directions", () => {
  assert.match(encoderAdvisories({ tune: "none", crf: 16 })[0].text, /very large/i);
  assert.match(encoderAdvisories({ tune: "none", crf: 28 })[0].text, /soft|banding|detail/i);
  const both = encoderAdvisories({ tune: "grain", crf: 16 });
  assert.equal(both.length, 2);
});
