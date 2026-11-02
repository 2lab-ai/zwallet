import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import assert from "node:assert/strict";

const source = readFileSync(
  new URL("../docs/assets/animation-player.js", import.meta.url),
  "utf8",
);
function player({ reduced = false } = {}) {
  let now = 0,
    nextId = 0;
  const frames = new Map(),
    listeners = new Map(),
    messages = [],
    draws = [],
    elements = new Map();
  const parent = { postMessage: (message) => messages.push(message) };
  const document = {
    body: { classList: { toggle() {} } },
    hidden: false,
    addEventListener() {},
    getElementById(id) {
      if (!elements.has(id))
        elements.set(id, {
          setAttribute() {},
          getAttribute() {
            return null;
          },
        });
      return elements.get(id);
    },
  };
  runInNewContext(source, {
    document,
    parent,
    location: { origin: "https://example.test", search: "?embed" },
    URLSearchParams,
    window: { duration: 36, draw: (t) => draws.push(t) },
    paths: [],
    lengths: {},
    tr: (_ko, en) => en,
    performance: { now: () => now },
    matchMedia: (query) => ({
      matches: query.includes("reduced-motion") && reduced,
      addEventListener() {},
    }),
    addEventListener: (type, fn) => listeners.set(type, fn),
    requestAnimationFrame: (fn) => {
      frames.set(++nextId, fn);
      return nextId;
    },
    cancelAnimationFrame: (id) => frames.delete(id),
  });
  return {
    messages,
    draws,
    frames,
    send(action, time, origin = "https://example.test", sender = parent) {
      listeners.get("message")({
        source: sender,
        origin,
        data: { type: "zwallet:control", action, time },
      });
    },
    advance(ms) {
      now += ms;
      const pending = [...frames.values()];
      frames.clear();
      pending.forEach((fn) => fn(now));
    },
  };
}

test("publishes the exact ended state even inside the 100ms message throttle", () => {
  const p = player();
  p.send("seek", 35.95);
  p.send("play");
  p.advance(60);
  assert.equal(p.messages.at(-1).time, 36);
  assert.equal(p.messages.at(-1).playing, false);
  assert.equal(p.frames.size, 0);
});

test("reduced-motion embeds start still and explicit play/pause remains usable", () => {
  const p = player({ reduced: true });
  assert.equal(p.frames.size, 0);
  p.send("play");
  p.advance(1000);
  assert.equal(p.draws.at(-1), 1);
  p.send("pause");
  const count = p.draws.length;
  p.advance(1000);
  assert.equal(p.draws.length, count);
  assert.equal(p.messages.at(-1).playing, false);
});

test("ignores foreign player messages and clamps only finite seek positions", () => {
  const p = player();
  const before = p.messages.length;
  p.send("play", undefined, "https://other.test");
  p.send("play", undefined, undefined, {});
  p.send("seek", NaN);
  p.send("seek", Infinity);
  assert.equal(p.messages.length, before);
  assert.equal(p.frames.size, 0);
  p.send("seek", 999);
  assert.equal(p.draws.at(-1), 36);
  p.send("seek", -10);
  assert.equal(p.draws.at(-1), 0);
});

test("retains a chapter selection while the lazy frame is still loading", () => {
  const messages = [],
    listeners = new Map(),
    elements = new Map();
  const element = () => ({
    classList: { toggle() {} },
    style: {},
    setAttribute() {},
    addEventListener() {},
  });
  const child = { postMessage: (message) => messages.push(message) };
  const frame = {
    ...element(),
    contentWindow: child,
    dataset: { src: "animation.html?embed" },
  };
  const chapters = [{ ...element(), dataset: { time: "22.2" } }];
  elements.set("story-frame", frame);
  const document = {
    body: element(),
    getElementById(id) {
      if (!elements.has(id)) elements.set(id, element());
      return elements.get(id);
    },
    querySelector() {
      return element();
    },
    querySelectorAll(selector) {
      return selector === "[data-time]" ? chapters : [];
    },
  };
  const site = readFileSync(
    new URL("../docs/assets/site.js", import.meta.url),
    "utf8",
  );
  runInNewContext(site, {
    document,
    location: { search: "", origin: "https://example.test" },
    URLSearchParams,
    matchMedia: () => ({ matches: false, addEventListener() {} }),
    IntersectionObserver: class {
      observe() {}
    },
    addEventListener: (type, fn) => listeners.set(type, fn),
  });
  chapters[0].onclick();
  assert.equal(messages.length, 1);
  assert.equal(messages[0].action, "state");
  listeners.get("message")({
    source: child,
    origin: "https://example.test",
    data: { type: "zwallet:state", time: 0, playing: false },
  });
  assert.equal(messages[1].action, "seek");
  assert.equal(messages[1].time, 22.2);
  assert.equal(messages[2].action, "pause");
});
