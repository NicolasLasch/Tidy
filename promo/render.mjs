// Frame-accurate renderer: drives headless Chrome over CDP, calling seek(t) per frame.
// usage: node render.mjs stills 1.0 2.5 ...   -> PNGs in ./out/still-<t>.png
//        node render.mjs video [audio.wav]     -> ./out/tidy-promo.mp4
import {spawn} from 'node:child_process';
import {mkdirSync, writeFileSync, existsSync} from 'node:fs';
import {fileURLToPath} from 'node:url';
import {dirname, join} from 'node:path';
import {homedir} from 'node:os';

const here = dirname(fileURLToPath(import.meta.url));
const out = join(here, 'out'); mkdirSync(out, {recursive: true});
const FPS = 30, DUR = 24.4, W = 1920, H = 1080;
const CHROME = join(homedir(), '.cache/puppeteer/chrome-headless-shell/mac_arm-137.0.7151.119/chrome-headless-shell-mac-arm64/chrome-headless-shell');
if (!existsSync(CHROME)) throw new Error('headless chrome not found at ' + CHROME);

const port = 9400 + Math.floor(Math.random() * 500);
const chrome = spawn(CHROME, [`--remote-debugging-port=${port}`, '--hide-scrollbars', '--allow-file-access-from-files', '--force-device-scale-factor=1', `--window-size=${W},${H}`, '--disable-gpu-vsync', 'about:blank'], {stdio: 'ignore'});
const sleep = ms => new Promise(r => setTimeout(r, ms));
let wsUrl;
for (let i = 0; i < 50; i++) { try { const j = await (await fetch(`http://127.0.0.1:${port}/json`)).json(); wsUrl = j.find(x => x.type === 'page')?.webSocketDebuggerUrl; if (wsUrl) break } catch {} await sleep(200); }
const ws = new WebSocket(wsUrl); await new Promise(r => ws.onopen = r);
let id = 0; const pend = new Map(); const logs = [];
ws.onmessage = e => { const m = JSON.parse(e.data); if (m.id && pend.has(m.id)) { pend.get(m.id)(m); pend.delete(m.id) } else if (m.method === 'Runtime.exceptionThrown') logs.push(JSON.stringify(m.params.exceptionDetails).slice(0, 600)); else if (m.method === 'Runtime.consoleAPICalled') logs.push(m.params.args.map(a => a.value).join(' ')) };
const cdp = (method, params = {}) => new Promise(r => { const i = ++id; pend.set(i, r); ws.send(JSON.stringify({id: i, method, params})) });
await cdp('Runtime.enable'); await cdp('Page.enable');
await cdp('Emulation.setDeviceMetricsOverride', {width: W, height: H, deviceScaleFactor: 1, mobile: false});
await cdp('Page.navigate', {url: 'file://' + join(here, 'index.html')});
await sleep(1500);
const ev = async expr => (await cdp('Runtime.evaluate', {expression: expr, awaitPromise: true, returnByValue: true})).result;
await ev('document.fonts.ready.then(()=>1)');
await ev('Promise.all([...document.images].map(i=>i.complete?1:new Promise(r=>i.onload=r)))');
const shot = async (t, fmt = 'png') => { const r = await ev(`seek(${t})`); if (r.exceptionDetails) throw new Error(JSON.stringify(r.exceptionDetails).slice(0, 800)); await sleep(15);
  return Buffer.from((await cdp('Page.captureScreenshot', {format: fmt, ...(fmt === 'jpeg' ? {quality: 94} : {})})).result.data, 'base64') };

const mode = process.argv[2];
try {
  if (mode === 'stills') {
    for (const t of process.argv.slice(3)) { writeFileSync(join(out, `still-${t}.png`), await shot(+t)); }
  } else if (mode === 'video') {
    const audio = process.argv[3];
    const args = ['-y', '-framerate', String(FPS), '-f', 'image2pipe', '-c:v', 'mjpeg', '-i', '-'];
    if (audio) args.push('-i', audio);
    args.push('-c:v', 'libx264', '-preset', 'slow', '-crf', '15', '-pix_fmt', 'yuv420p', '-movflags', '+faststart');
    if (audio) args.push('-c:a', 'aac', '-b:a', '256k', '-shortest');
    args.push(join(out, 'tidy-promo.mp4'));
    const ff = spawn('ffmpeg', args, {stdio: ['pipe', 'ignore', 'inherit']});
    const n = Math.round(FPS * DUR);
    for (let f = 0; f < n; f++) { const buf = await shot(f / FPS, 'jpeg'); if (!ff.stdin.write(buf)) await new Promise(r => ff.stdin.once('drain', r)); if (f % 60 === 0) console.log('frame', f, '/', n) }
    ff.stdin.end(); await new Promise(r => ff.on('close', r));
    console.log('done');
  }
} finally { if (logs.length) console.log('PAGE LOGS:\n' + logs.join('\n')); ws.close(); chrome.kill() }
