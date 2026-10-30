import {spawn, execFileSync} from 'node:child_process';
import {mkdtemp, readFile, writeFile, mkdir, rm, access} from 'node:fs/promises';
import {tmpdir} from 'node:os';
import {join, resolve} from 'node:path';
import {fileURLToPath} from 'node:url';
import {once} from 'node:events';

// Node 22+ and Chrome/Chromium. All frames are drawn from one deterministic SVG timeline.
const args = process.argv.slice(2);
const value = (flag, fallback) => args.includes(flag) ? args[args.indexOf(flag) + 1] : fallback;
const lang = value('--lang', 'en');
if (!['en', 'ko'].includes(lang)) throw new Error('--lang must be en or ko');
const stillsOnly = args.includes('--stills-only');
const output = resolve(value('--output', fileURLToPath(new URL('../../docs/assets/', import.meta.url))));
const review = resolve(value('--review', join(tmpdir(), `zwallet-flow-${lang}`)));
await mkdir(output, {recursive: true});
await mkdir(review, {recursive: true});
let chromePath = process.env.CHROME;
if (!chromePath) {
  for (const candidate of ['/Applications/Google Chrome.app/Contents/MacOS/Google Chrome', '/usr/bin/google-chrome', '/usr/bin/chromium', '/usr/bin/chromium-browser']) {
    try { await access(candidate); chromePath = candidate; break; } catch { /* Try next installed browser. */ }
  }
}
if (!chromePath) throw new Error('Set CHROME to the Chrome/Chromium executable.');
const ffmpeg = stillsOnly ? null : process.env.FFMPEG || execFileSync(process.env.PYTHON || 'python3', ['-c', 'import imageio_ffmpeg; print(imageio_ffmpeg.get_ffmpeg_exe())'], {encoding: 'utf8'}).trim();
const profile = await mkdtemp(join(tmpdir(), 'zwt-film-'));
const chrome = spawn(chromePath, ['--headless=new', '--disable-background-timer-throttling', '--disable-renderer-backgrounding', '--remote-debugging-port=0', '--user-data-dir=' + profile, '--no-first-run', '--no-default-browser-check', 'about:blank'], {stdio: 'ignore'});
let ws, encoder;
const delay = ms => new Promise(r => setTimeout(r, ms));
try {
  let port;
  for (let i = 0; i < 100; i++) {
    try { port = Number((await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]); break; } catch { await delay(100); }
  }
  if (!port) throw new Error('Chrome did not start. Check local browser permissions.');
  const tabs = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  ws = new WebSocket(tabs.find(t => t.type === 'page').webSocketDebuggerUrl);
  await new Promise((r, j) => { ws.onopen = r; ws.onerror = j; });
  let id = 0;
  const pending = new Map();
  ws.onmessage = e => {
    const m = JSON.parse(e.data), call = pending.get(m.id);
    if (call) { clearTimeout(call.timer); pending.delete(m.id); m.error ? call.reject(new Error(JSON.stringify(m.error))) : call.resolve(m.result); }
  };
  function send(method, params = {}) {
    return new Promise((resolve, reject) => {
      const next = ++id, timer = setTimeout(() => { pending.delete(next); reject(new Error('Browser timeout: ' + method)); }, 30000);
      pending.set(next, {resolve, reject, timer});
      ws.send(JSON.stringify({id: next, method, params}));
    });
  }
  async function evaluate(expression) {
    const r = await send('Runtime.evaluate', {expression, awaitPromise: true, returnByValue: true});
    if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails));
    return r.result.value;
  }
  await send('Page.enable'); await send('Runtime.enable'); await send('Page.bringToFront');
  await send('Emulation.setDeviceMetricsOverride', {width: 1600, height: 1000, deviceScaleFactor: 1, mobile: false});
  const source = new URL('motion.html', import.meta.url); source.search = `render&lang=${lang}`;
  await send('Page.navigate', {url: source.href});
  let ready = false;
  for (let n = 0; n < 100; n++) {
    if (await evaluate('typeof window.draw === "function"')) { ready = true; break; }
    await delay(100);
  }
  if (!ready) throw new Error('Timeline did not load.');
  await evaluate('document.fonts.ready.then(() => true)');
  const duration = await evaluate('window.duration');
  async function frame(t) {
    await evaluate(`window.draw(${t})`);
    // Wait for actual paint; a screenshot immediately after mutation can capture stale state.
    await evaluate('new Promise(r => requestAnimationFrame(() => requestAnimationFrame(() => r(true))))');
    return Buffer.from((await send('Page.captureScreenshot', {format: 'png'})).data, 'base64');
  }
  const stem = lang === 'ko' ? 'agent-story-ko' : 'agent-story';
  for (const t of [1.8, 3.7, 6.5, 8.5, 12.7, 15.9, 18, 22.8, 25.8, 29, 34]) {
    await writeFile(join(review, `frame-${t}.png`), await frame(t));
  }
  await writeFile(join(output, `${stem}-poster.png`), await frame(12.7));
  if (!stillsOnly) {
    encoder = spawn(ffmpeg, ['-y', '-loglevel', 'error', '-f', 'image2pipe', '-vcodec', 'png', '-framerate', '60', '-i', 'pipe:0', '-an', '-c:v', 'libx264', '-preset', 'fast', '-crf', '19', '-pix_fmt', 'yuv420p', '-movflags', '+faststart', join(output, stem + '.mp4')], {stdio: ['pipe', 'ignore', 'inherit']});
    let encoderError;
    encoder.on('error', error => { encoderError = error; });
    encoder.stdin.on('error', error => { encoderError = error; });
    const done = new Promise(resolve => encoder.on('close', code => resolve(code)));
    for (let n = 0; n < duration * 60; n++) {
      if (encoderError) throw encoderError;
      if (encoder.exitCode !== null) throw new Error('Encoder stopped early: ' + encoder.exitCode);
      if (!encoder.stdin.write(await frame(n / 60))) await once(encoder.stdin, 'drain');
      if (n % 600 === 0) console.log(`${lang}: ${n / 60}/${duration}s rendered`);
    }
    encoder.stdin.end();
    const code = await done;
    if (code !== 0 || encoderError) throw encoderError || new Error('Encoder exit ' + code);
  }
  console.log(JSON.stringify({language: lang, duration, fps: 60, frames: duration * 60, size: [1600, 1000], output, review, stillsOnly}));
} finally {
  if (encoder && encoder.exitCode === null) encoder.kill('SIGTERM');
  if (ws) ws.close();
  chrome.kill('SIGTERM');
  await delay(500);
  await rm(profile, {recursive: true, force: true});
}
