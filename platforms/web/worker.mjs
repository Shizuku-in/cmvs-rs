import init, { WebRuntime, identify } from './pkg/cmvs_web.js';
import { BrowserFiles } from './filesystem.mjs';
import { AudioBridge } from './audio-bridge.mjs';
const audio = new AudioBridge();
globalThis.cmvsAudio = audio;
function flushAudio() {
  const commands=audio.take();
  if(commands.length) postMessage({type:'audio',commands},commands.filter(c=>c.pcm).map(c=>c.pcm.buffer));
}

const reader = new FileReaderSync();
const files = new BrowserFiles(blob => new Uint8Array(reader.readAsArrayBuffer(blob)));
globalThis.cmvsFs = files;
let runtime, database, saveKey, busy = false, lastFrame = 0, useGpu = false;
const held = new Set(), pulses = new Set();
let pointer = [-1, -1], focused = true;
function transaction(mode, work) {
  return new Promise((resolve, reject) => {
    const tx = database.transaction('saves', mode);
    const result = work(tx.objectStore('saves'));
    tx.oncomplete = () => resolve(result?.result);
    tx.onerror = () => reject(tx.error);
    tx.onabort = () => reject(tx.error || Error('Save transaction aborted'));
  });
}
async function persist() {
  if (!files.dirty) return;
  await transaction('readwrite', store => store.put(files.snapshot(), saveKey));
  files.dirty = false;
}
async function start(message) {
  postMessage({ type: 'status', text: '正在识别游戏并加载 wasm…' });
  for (const {path, file} of message.files) {
    if (path) files.mount(path, file);
  }
  await init();
  saveKey = identify(message.profile);
  if (!navigator.locks) throw Error('This browser needs Web Locks support (use localhost or HTTPS)');
  await new Promise((resolve, reject) => {
    navigator.locks.request('cmvs-web-' + saveKey, {ifAvailable:true}, async lock => {
      if (!lock) { reject(Error('This game is already open in another tab')); return; }
      resolve();
      await new Promise(() => {}); // Terminating the worker releases ownership.
    }).catch(reject);
  });
  database = await new Promise((resolve, reject) => {
    const request = indexedDB.open('cmvs-web', 1);
    request.onupgradeneeded = () => request.result.createObjectStore('saves');
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
  let saved = await transaction('readonly', store => store.get(saveKey)) || [];
  if (message.backup) {
    if (message.backup.size > 64 * 1024 * 1024) throw Error('Save JSON exceeds 64 MiB');
    const backup = JSON.parse(await message.backup.text());
    if (backup.version !== 1 || backup.saveKey !== saveKey) throw Error('Save backup belongs to another game');
    saved = backup.entries.map(([path,bytes]) => {
      if (!Array.isArray(bytes) || bytes.some(n => !Number.isInteger(n) || n < 0 || n > 255)) throw Error('Invalid save bytes');
      return [path,new Uint8Array(bytes)];
    });
  }
  files.restore(saved);
  runtime = new WebRuntime(new Uint8Array(await message.font.arrayBuffer()), message.face, message.profile, message.skip);
  useGpu = Boolean(message.canvas);
  if (useGpu) await runtime.enable_gpu(message.canvas);
  lastFrame = performance.now();
  postMessage({ type: 'ready', width: runtime.width(), height: runtime.height(), saveKey });
}
async function tick() {
  if (!runtime || busy) return;
  busy = true;
  try {
    const now = performance.now();
    const date = new Date();
    runtime.set_clock(new Uint16Array([date.getFullYear(),date.getMonth()+1,date.getDate(),date.getHours(),date.getMinutes(),date.getSeconds()]));
    const keys = new Uint8Array([...held, ...pulses]);
    if (runtime.needs_input()) pulses.clear();
    const result = useGpu
      ? await runtime.tick_gpu(Math.round(now - lastFrame), keys, ...pointer, focused)
      : runtime.tick(Math.round(now - lastFrame), keys, ...pointer, focused);
    flushAudio();
    if (result === 1) {
      lastFrame = now;
      const pixels = useGpu ? undefined : runtime.pixels();
      postMessage({ type: 'frame', width: runtime.width(), height: runtime.height(), pixels, renderMode: runtime.render_mode() }, pixels ? [pixels.buffer] : []);
    }
    if (result === 2) runtime.save_preferences();
    await persist();
    if (result === 2) { postMessage({ type: 'quit' }); runtime.free(); runtime = null; }
    else postMessage({ type: 'next', delay: result === 0 ? 0 : result === 3 ? 100 : 16 });
  } finally { busy = false; }
}
async function handleMessage(data) {
  try {
    if (data.type === 'start') await start(data);
    else if (data.type === 'tick') await tick();
    else if (data.type === 'audio-clock') audio.report(data.clock);
    else if (data.type === 'input') {
      if (data.key !== undefined) data.down ? held.add(data.key) : held.delete(data.key);
      if (data.pulse !== undefined) pulses.add(data.pulse);
      if (data.pointer) pointer = data.pointer;
      if (data.focused !== undefined) {
        focused = data.focused;
        if (!focused) { held.clear(); pulses.clear(); pointer = [-1, -1]; }
      }
    } else if (data.type === 'stop') {
      if (runtime) runtime.save_preferences();
      await persist();
      postMessage({type:'stopped'});
    } else if (data.type === 'export') {
      await persist();
      postMessage({ type: 'export', saveKey, entries: files.snapshot() });
    }
  } catch (error) {
    postMessage({ type: 'error', text: String(error?.stack || error) });
  }
}
// Input and audio clocks remain live while mapping a screenshot. Runtime mutations
// wait for the current tick to release its wasm borrow before stop/export can run.
let pending = Promise.resolve();
onmessage = ({data}) => {
  if (data.type === 'input' || data.type === 'audio-clock') void handleMessage(data);
  else pending = pending.then(() => handleMessage(data));
};
