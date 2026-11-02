(() => {
  const ko = new URLSearchParams(location.search).get("lang") === "ko";
  const tr = (en, kr) => (ko ? kr : en);
  const $ = (id) => document.getElementById(id);
  if (ko) {
    document.documentElement.lang = "ko";
    document.title = "zwallet — 내 에이전트. 내 돈. 내가 정한 규칙.";
    document.querySelectorAll("[data-ko]").forEach((el) => {
      el.textContent = el.dataset.ko;
    });
    $("language").textContent = "English";
    $("language").lang = "en";
    $("language").href = "?lang=en";
    $("language").setAttribute("aria-label", "Read in English");
    document.querySelector("a.wordmark").href = "?lang=ko";
    $("story-frame").title = "zwallet 작동 원리 애니메이션";
    document
      .querySelectorAll(".film-link")
      .forEach((el) => (el.href = "film.html?lang=ko"));
    document.querySelector('meta[name="description"]').content =
      "에이전트에게 작업비를 맡기고 통제권은 유지하세요. 내 기기의 키와 직접 정한 결제 규칙으로 사용하는 오픈소스 지갑.";
  }
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  let paused = reduced.matches;
  function setMotion() {
    document.body.classList.toggle("paused", paused);
    $("motion-toggle").disabled = reduced.matches;
    $("motion-toggle").setAttribute("aria-pressed", String(paused));
    $("motion-toggle").textContent = reduced.matches
      ? tr("Reduced motion", "동작 줄이기 적용 중")
      : paused
        ? tr("Resume motion", "모션 재생")
        : tr("Pause motion", "모션 정지");
  }
  $("motion-toggle").onclick = () => {
    paused = !paused;
    setMotion();
  };
  reduced.addEventListener("change", () => {
    paused = reduced.matches;
    setMotion();
  });
  setMotion();
  const art = document.querySelector(".hero-art");
  new IntersectionObserver((entries) => {
    art.style.animationPlayState = entries[0].isIntersecting
      ? "running"
      : "paused";
    art.classList.toggle("paused", !entries[0].isIntersecting);
  }).observe(art);
  $("copy").onclick = async () => {
    try {
      await navigator.clipboard.writeText($("install-command").textContent);
      $("copy").textContent = tr("Copied", "복사됨");
      $("copy-status").textContent = tr(
        "Install command copied.",
        "설치 명령을 복사했습니다.",
      );
    } catch {
      const range = document.createRange();
      range.selectNodeContents($("install-command"));
      const selection = getSelection();
      selection.removeAllRanges();
      selection.addRange(range);
      $("copy-status").textContent = tr(
        "Select and copy the command manually.",
        "선택된 명령을 직접 복사해 주세요.",
      );
    }
  };
  const frame = $("story-frame");
  let playing = false,
    ready = false,
    visible = false,
    started = false,
    chapter = -1;
  const pending = { time: null, playback: null };
  const send = (action, time) => {
    if (!ready && action !== "state") {
      if (action === "seek") pending.time = time;
      if (action === "play" || action === "pause") pending.playback = action;
      return;
    }
    frame.contentWindow?.postMessage(
      { type: "zwallet:control", action, time },
      location.origin,
    );
  };
  const descriptions = [
    [
      "Fund a dedicated wallet with 50 USDT and gas, choose the payees, set a limit per payment and an access window. Give your agent the task.",
      "전용 지갑에 50 USDT와 가스비를 넣고, 수취인·거래별 한도·접근 시간을 정합니다. 에이전트에게는 작업을 맡깁니다.",
    ],
    [
      "The agent completes a permitted 2 USDT payment, checks the receipt and reports the payment, gas and remaining funds. This report is assembled by the agent.",
      "에이전트가 허용된 2 USDT 결제를 완료하고 영수증을 확인한 뒤 결제액·가스비·잔액을 보고합니다. 이 보고는 에이전트가 작성합니다.",
    ],
    [
      "A 20 USDT request exceeds the configured 5 USDT limit per payment. The wallet refuses to sign. Repeated payments within the limit can still spend the balance.",
      "20 USDT 요청이 설정된 1회 5 USDT 한도를 넘었습니다. 지갑은 서명을 거절합니다. 한도 안의 결제가 반복되면 잔액을 소진할 수 있습니다.",
    ],
    [
      "Lock the wallet or let access expire to stop new signatures. Lock ends all zwallet signing sessions for your OS user; it does not reverse a transaction already sent.",
      "직접 잠그거나 접근 시간이 만료되면 새 서명이 끝납니다. 잠금은 같은 OS 사용자의 모든 zwallet 서명 세션을 끝내며, 이미 전송한 거래를 취소하지는 않습니다.",
    ],
    [
      "WIP: services, shopping and investing under your instructions. These adapters are not implemented. Investing can result in losses.",
      "WIP: 내가 지시한 서비스 결제·구매·투자로 이어지는 구상입니다. 이 연동은 아직 구현되지 않았으며 투자는 손실이 발생할 수 있습니다.",
    ],
  ];
  const chapters = [...document.querySelectorAll("[data-time]")];
  function state(time, isPlaying) {
    playing = isPlaying;
    $("play").textContent = playing
      ? tr("Pause", "일시 정지")
      : time >= 36
        ? tr("Replay", "다시 보기")
        : tr("Play", "재생");
    $("seek").value = time;
    $("story-time").textContent =
      `0:${String(Math.floor(time)).padStart(2, "0")} / 0:36`;
    const next =
      time < 11.3 ? 0 : time < 14.5 ? 1 : time < 20 ? 2 : time < 24.5 ? 3 : 4;
    if (next !== chapter) {
      chapter = next;
      chapters.forEach((el, i) =>
        el.setAttribute("aria-pressed", String(i === next)),
      );
      $("scene-copy").textContent = tr(...descriptions[next]);
    }
  }
  const maybeStart = () => {
    if (ready && visible && !started && !reduced.matches) {
      started = true;
      send("play");
    }
  };
  addEventListener("message", (event) => {
    if (
      event.source !== frame.contentWindow ||
      event.origin !== location.origin ||
      event.data?.type !== "zwallet:state"
    )
      return;
    const { time, playing: isPlaying } = event.data;
    if (
      !Number.isFinite(time) ||
      time < 0 ||
      time > 36 ||
      typeof isPlaying !== "boolean"
    )
      return;
    const firstState = !ready;
    ready = true;
    state(time, isPlaying);
    if (firstState) {
      if (pending.time !== null) send("seek", pending.time);
      if (pending.playback !== null) send(pending.playback);
    }
    maybeStart();
  });
  frame.addEventListener("load", () => send("state"));
  // Install message handlers before starting the one selected-language frame.
  frame.src = `${frame.dataset.src}&lang=${ko ? "ko" : "en"}`;
  send("state");
  new IntersectionObserver(
    (entries) => {
      visible = entries[0].isIntersecting;
      if (!visible) send("pause");
      else maybeStart();
    },
    { threshold: 0.2 },
  ).observe(frame);
  $("play").onclick = () => {
    started = true;
    send(playing ? "pause" : "play");
  };
  $("replay").onclick = () => {
    started = true;
    send("seek", 0);
    send("play");
  };
  $("seek").oninput = (e) => {
    started = true;
    send("seek", Number(e.target.value));
  };
  chapters.forEach(
    (el) =>
      (el.onclick = () => {
        started = true;
        send("pause");
        send("seek", Number(el.dataset.time));
      }),
  );
  reduced.addEventListener("change", () => {
    if (reduced.matches) send("pause");
  });
})();
