import { AudioDevice } from './audio-device.mjs';
const $ = id => document.getElementById(id);
let canvas = $('canvas'), context, logicalSize = [canvas.width, canvas.height];
let worker, timer, saveKey, audio, audioTimer, launch=0;
const physical = new Map();
const keyCodes = {Backspace:20,Tab:21,Enter:22,NumpadEnter:22,ShiftLeft:23,ShiftRight:23,ControlLeft:24,ControlRight:24,Escape:25,Space:26,PageUp:27,PageDown:28,End:29,Home:30,Insert:31,Delete:32,ArrowUp:33,ArrowDown:34,ArrowLeft:35,ArrowRight:36,NumpadMultiply:47,NumpadAdd:48,NumpadSubtract:49,NumpadDecimal:50,NumpadDivide:51,Semicolon:112,Equal:113,Comma:114,Minus:115,Period:116,Slash:117,Backquote:118,BracketLeft:119,Backslash:120,BracketRight:121,Quote:122};
for (let i=0;i<10;i++) { keyCodes['Numpad'+i]=37+i; keyCodes['Digit'+i]=76+i; }
for (let i=1;i<=24;i++) keyCodes['F'+i]=51+i;
for (let i=0;i<26;i++) keyCodes['Key'+String.fromCharCode(65+i)]=86+i;
function send(message, transfer = []) { worker?.postMessage(message, transfer); }
function status(text) { $('status').textContent = text; }
function stop(text) {
  launch++;
  clearTimeout(timer); clearInterval(audioTimer);
  if(audio) {void audio.close(); audio=undefined;}
  worker?.terminate(); worker = undefined; physical.clear();
  $('start').disabled = false; $('stop').disabled = true; $('export').disabled = true;
  if (text) status(text);
}
$('setup').onsubmit = async event => {
  event.preventDefault();
  stop();
  const current=launch;
  const renderer=$('renderer').value;
  $('start').disabled=true; $('stop').disabled=false;
  try {
    const context=new AudioContext({latencyHint:'interactive'});
    audio=new AudioDevice(context);
    await context.resume();
    if(current!==launch) return;
    if(context.state!=='running') throw Error('浏览器未启用音频，请再次点击启动');
  } catch(error) {if(current===launch) stop('音频初始化失败：'+error);return;}
  try {
    if(renderer==='webgpu' && (!navigator.gpu || !canvas.transferControlToOffscreen)) throw Error('此浏览器不支持 WebGPU Worker，请选择 CPU 或使用支持 WebGPU 的浏览器');
    // A transferred canvas cannot be reused, including after stopping or changing backends.
    const replacement=canvas.cloneNode(false);
    canvas.replaceWith(replacement);canvas=replacement;bindCanvas();
    context=renderer==='cpu'?canvas.getContext('2d'):undefined;
    worker = new Worker('./worker.mjs', {type:'module'});
  }
  catch(error) {stop('启动失败：'+error);return;}
  audioTimer=setInterval(()=>send({type:'audio-clock',clock:audio.clock()}),25);
  $('start').disabled = true; $('stop').disabled = false;
  worker.onerror = event => stop('运行失败：' + event.message);
  worker.onmessage = ({data}) => {
    if(data.type==='audio') {
      try {for(const command of data.commands) audio.apply(command);}
      catch(error) {stop('音频输出失败：'+error);}
    }
    else if (data.type === 'status') status(data.text);
    else if (data.type === 'error') stop('运行停止：' + data.text);
    else if (data.type === 'ready') {
      logicalSize=[data.width,data.height];
      if(renderer==='cpu') {canvas.width = data.width; canvas.height = data.height;}
      saveKey = data.saveKey;
      $('export').disabled = false; canvas.focus();
      status(`运行中 · ${renderer==='webgpu'?'WebGPU':'CPU'} · 存档自动保存在当前浏览器`); send({type:'tick'});
    } else if (data.type === 'frame') {
      canvas.style.imageRendering = data.renderMode === 1 ? 'auto' : 'pixelated';
      if(data.pixels) context.putImageData(new ImageData(new Uint8ClampedArray(data.pixels.buffer),data.width,data.height),0,0);
    } else if (data.type === 'next') timer = setTimeout(() => send({type:'tick'}),data.delay);
    else if (data.type === 'stopped') stop('已停止，存档和设置已保存。');
    else if (data.type === 'quit') stop('游戏已退出，存档已保存。');
    else if (data.type === 'export') {
      const entries = data.entries.map(([path,bytes]) => [path,Array.from(bytes)]);
      const url = URL.createObjectURL(new Blob([JSON.stringify({version:1,saveKey,entries})],{type:'application/json'}));
      const link = document.createElement('a'); link.href=url; link.download='cmvs-saves.json'; link.click();
      setTimeout(() => URL.revokeObjectURL(url),1000); status('存档已导出 · 运行中');
    }
  };
  try {
    const offscreen=renderer==='webgpu'?canvas.transferControlToOffscreen():undefined;
    send({type:'start',canvas:offscreen,files:[...$('game').files].map(file=>({path:file.webkitRelativePath.split('/').slice(1).join('/'),file})),font:$('font').files[0],face:Number($('face').value),profile:$('profile').value.trim(),skip:$('skip').checked,backup:$('backup').files[0]},offscreen?[offscreen]:[]);
  } catch(error) {stop('启动失败：'+error);}
};
$('stop').onclick = () => {
  if(!worker) {stop('已停止。');return;}
  clearTimeout(timer); $('stop').disabled=true; send({type:'stop'});
};
$('export').onclick = () => send({type:'export'});
$('fullscreen').onclick = () => $('stage').requestFullscreen().catch(error => status(String(error)));
function point(event) {
  const bounds = canvas.getBoundingClientRect();
  return [Math.floor((event.clientX-bounds.left)*logicalSize[0]/bounds.width),Math.floor((event.clientY-bounds.top)*logicalSize[1]/bounds.height)];
}
function bindCanvas() {
  canvas.oncontextmenu = event => event.preventDefault();
  canvas.onpointermove = event => send({type:'input',pointer:point(event)});
  canvas.onpointerdown = event => {
    event.preventDefault(); void audio?.context.resume(); canvas.focus(); canvas.setPointerCapture(event.pointerId);
    const key = event.button === 2 ? 3 : 1;
    send({type:'input',pointer:point(event),key,down:true,pulse:key});
  };
  canvas.onpointerup = event => send({type:'input',pointer:point(event),key:event.button===2?3:1,down:false});
  canvas.onpointercancel = () => send({type:'input',focused:false});
  canvas.addEventListener('wheel',event => {event.preventDefault(); send({type:'input',pulse:event.deltaY<0?4:5});},{passive:false});
  canvas.onkeydown = event => {
    const key = keyCodes[event.code]; if (!key) return;
    event.preventDefault();
    if (!physical.has(event.code)) { physical.set(event.code,key); send({type:'input',key,down:true,pulse:key}); }
  };
  canvas.onblur = () => {physical.clear();send({type:'input',focused:false});};
  canvas.onfocus = () => send({type:'input',focused:true});
}
bindCanvas();
window.addEventListener('keyup',event => {
  const key = physical.get(event.code); physical.delete(event.code);
  if (key && ![...physical.values()].includes(key)) send({type:'input',key,down:false});
});
window.addEventListener('blur',() => {physical.clear();send({type:'input',focused:false});});
window.addEventListener('focus',() => send({type:'input',focused:document.activeElement===canvas}));
