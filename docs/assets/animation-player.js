// Playback is separate from the deterministic film timeline in animation.html.
(() => {
  const params = new URLSearchParams(location.search);
  const embedded = params.has("embed"),
    render = params.has("render");
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  let playing = !embedded && !render && !reduced.matches;
  let position = 0,
    start = performance.now(),
    raf = 0,
    lastNotice = -1;
  const scene = document.getElementById("scene");
  const originalDraw = window.draw;
  const mobile = matchMedia("(max-width: 640px)");
  // Capture desktop attributes before reflow; resizing back is lossless.
  const originals = new Map();
  const attrs = (id, values) => {
    const el = document.getElementById(id);
    for (const [key, value] of Object.entries(values)) {
      const token = id + ":" + key;
      if (!originals.has(token))
        originals.set(token, [el, key, el.getAttribute(key)]);
      el.setAttribute(key, value);
    }
  };
  const narrow = () => embedded && mobile.matches && !render;
  const route = (id, d) => attrs(id, { d });
  function layout() {
    for (const [el, key, value] of originals.values()) {
      value === null ? el.removeAttribute(key) : el.setAttribute(key, value);
    }
    document.body.classList.toggle("narrow", narrow());
    if (narrow()) {
      scene.setAttribute("viewBox", "0 0 640 1200");
      for (const [id, x, y] of [
        ["you", 160, 240],
        ["agent", 160, 510],
        ["wallet", 460, 510],
        ["payee", 460, 780],
      ])
        attrs(id, { transform: `translate(${x} ${y})` });
      const routes = {
        fund: "M 160 182 V 110 Q 160 90 180 90 H 440 Q 460 90 460 110 V 452",
        task: "M 160 298 V 452",
        req: "M 218 510 H 402",
        pay: "M 460 568 V 722",
        result: "M 402 780 H 70 Q 48 780 48 758 V 532 Q 48 510 70 510 H 102",
        report: "M 102 510 H 70 Q 48 510 48 488 V 262 Q 48 240 70 240 H 102",
      };
      for (const [key, d] of Object.entries(routes)) {
        route(key + "-light", d);
        const base = { req: "request", pay: "payment" }[key] || key;
        if (document.getElementById(base + "-path")) route(base + "-path", d);
      }
      attrs("balance", { transform: "translate(460 270)" });
      attrs("rules", { transform: "translate(265 385) scale(.85)" });
      attrs("safety-note", { transform: "translate(28 985) scale(.88)" });
      const cards = document.querySelectorAll("#future > g[transform]");
      cards.forEach((el, i) => {
        if (!el.id) el.id = "mobile-future-" + i;
        attrs(el.id, {
          transform: `translate(90 ${690 + i * 145}) scale(1.2)`,
        });
      });
      document
        .querySelectorAll("#future > g:first-child path")
        .forEach((el, i) => {
          if (!el.id) el.id = "mobile-future-path-" + i;
          attrs(el.id, { d: `M 460 568 V 650 H 55 V ${755 + i * 145} H 90` });
        });
      attrs("future-note", { x: 320, y: 1160, "font-size": 22 });
    } else scene.setAttribute("viewBox", "0 0 1600 1000");
    for (const key of paths)
      lengths[key] = document.getElementById(key + "-light").getTotalLength();
    draw(position);
  }
  function notice(force = false) {
    if (!embedded || (!force && Math.abs(position - lastNotice) < 0.1)) return;
    lastNotice = position;
    parent.postMessage(
      { type: "zwallet:state", time: position, playing },
      location.origin,
    );
  }
  function draw(t) {
    originalDraw(t);
    if (narrow()) {
      const lift = 24 * (1 - ease((t - 11.3) / 0.65));
      document
        .getElementById("receipt")
        .setAttribute("transform", `translate(28 ${970 + lift}) scale(.88)`);
    }
  }
  function tick(now) {
    raf = 0;
    if (!playing) return;
    position = Math.min(window.duration, position + (now - start) / 1000);
    start = now;
    draw(position);
    if (position >= window.duration) playing = false;
    notice(!playing);
    updateButton();
    if (playing) raf = requestAnimationFrame(tick);
  }
  function updateButton() {
    document.getElementById("toggle").textContent = playing
      ? tr("일시 정지", "Pause")
      : tr("재생", "Play");
  }
  function pause() {
    playing = false;
    cancelAnimationFrame(raf);
    raf = 0;
    updateButton();
    notice(true);
  }
  function play() {
    if (render) return;
    if (position >= window.duration) position = 0;
    playing = true;
    start = performance.now();
    if (!raf) raf = requestAnimationFrame(tick);
    updateButton();
    notice(true);
  }
  function seek(value) {
    if (!Number.isFinite(value)) return;
    position = Math.max(0, Math.min(window.duration, value));
    start = performance.now();
    draw(position);
    notice(true);
  }
  document.body.classList.toggle("embed", embedded);
  document.body.classList.toggle("render", render);
  document.getElementById("toggle").onclick = () =>
    playing ? pause() : play();
  document.getElementById("replay").onclick = () => {
    seek(0);
    play();
  };
  document.getElementById("replay").textContent = tr("다시 보기", "Replay");
  document
    .getElementById("seek")
    .setAttribute("aria-label", tr("재생 위치", "Playback position"));
  document.getElementById("seek").oninput = (e) => seek(Number(e.target.value));
  addEventListener("message", (event) => {
    if (
      !embedded ||
      event.source !== parent ||
      event.origin !== location.origin ||
      event.data?.type !== "zwallet:control"
    )
      return;
    if (event.data.action === "play") play();
    if (event.data.action === "pause") pause();
    if (event.data.action === "seek") seek(event.data.time);
    if (event.data.action === "state") notice(true);
  });
  document.addEventListener("visibilitychange", () => {
    if (document.hidden) pause();
  });
  reduced.addEventListener("change", () => {
    if (reduced.matches) pause();
  });
  mobile.addEventListener("change", layout);
  // Rendering still calls the unchanged deterministic draw function directly.
  layout();
  updateButton();
  notice(true);
  if (playing) raf = requestAnimationFrame(tick);
})();
