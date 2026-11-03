// Runs the actual generated wasm with synthetic PS2A/CPZ fixtures, without game data.
import assert from 'node:assert/strict';
import {readFileSync, existsSync} from 'node:fs';
import init, {WebRuntime, identify} from './pkg/cmvs_web.js';
import {BrowserFiles} from './filesystem.mjs';
const fs=new BrowserFiles(bytes => bytes);
const mount=(path,bytes) => fs.mount(path,{size:bytes.length,slice:(start,end)=>bytes.slice(start,end)});
const words=[];
function command(op,args=[]) {
  for (const arg of [...args].reverse()) words.push(0x40,0x04,4,0,arg&255,(arg>>>8)&255,(arg>>>16)&255,(arg>>>24)&255);
  words.push(op&255,op>>>8);
}
// More than one cooperative budget, followed by a first-launch system save.
for(let i=0;i<2100;i++) command(0x201e,[1]);
command(0x201d,[0x12345678]);
command(0x2091,[0,2048,0]); command(0x2092,[0,2048,0]);
command(0x2093,[0,1024,0]); command(0x2094,[0,64,0]);
command(0x2128); command(0x2092,[2400,1,96]); command(0x2129);
command(0x2000); command(0x2001);
const code=new Uint8Array(words);
const raw=new Uint8Array(49+code.length), view=new DataView(raw.buffer);
raw.set(new TextEncoder().encode('PS2A'));
[48,0,0,0,code.length,0,1,0,0,code.length+1,0].forEach((n,i)=>view.setUint32(4+i*4,n,true));
raw.set(code.map(b => (((b<<1)|(b>>7))+124)&255),48);
raw[48+code.length]=124; // NUL text string
mount('data/pack/start.ps3',raw);
mount('data/pack/script.cpz',new Uint8Array(Buffer.from(readFileSync(new URL('../../tests/fixtures/cpz6-script.hex',import.meta.url),'utf8').trim(),'hex')));
mount('cmvs.cfg',new TextEncoder().encode('[WINDOW_RESOLVING]\nBASE=64,48\n'));
globalThis.cmvsFs=fs;
await init({module_or_path:readFileSync(new URL('./pkg/cmvs_web_bg.wasm',import.meta.url))});
const profile='cmvs-20160201';
console.log("Identifying synthetic game");
assert.match(identify(profile),/^cmvs-20160201-/);
const candidates=[process.env.CMVS_TEST_FONT,'C:/Windows/Fonts/msgothic.ttc','/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf'].filter(Boolean);
const font=candidates.find(existsSync); if(!font) throw Error('Set CMVS_TEST_FONT to a TTF/OTF/TTC file');
console.log("Creating runtime");
const runtime=new WebRuntime(new Uint8Array(readFileSync(font)),0,profile,false);
assert.equal(runtime.needs_input(),true);
runtime.set_clock(new Uint16Array([2026,1,1,12,0,0]));
assert.equal(runtime.tick(16,new Uint8Array([1]),8,8,true),0);
assert.equal(runtime.needs_input(),false);
assert.throws(() => runtime.pixels(),/no completed frame/);
assert.equal(runtime.tick(16,new Uint8Array(),-1,-1,true),1);
assert.equal(runtime.width(),64); assert.equal(runtime.height(),48);
assert.equal(new TextDecoder().decode(fs.read('/user/system.dat',0,4)),'CSS1');
assert.equal(runtime.save_preferences(),true);
assert.ok(fs.size('/user/cmvs.cfg')>0);
assert.equal(runtime.pixels().length,runtime.width()*runtime.height()*4);
assert.equal(runtime.tick(16,new Uint8Array(),-1,-1,true),2);
runtime.free();
const saves=fs.snapshot();
assert.ok(saves.some(([path])=>path==='/user/system.dat'));
fs.restore(saves);
const resumed=new WebRuntime(new Uint8Array(readFileSync(font)),0,profile,false);
assert.equal(resumed.tick(16,new Uint8Array(),-1,-1,true),0);
assert.equal(resumed.tick(16,new Uint8Array(),-1,-1,true),1);
assert.ok(fs.size('/user/system.bak')>0);
resumed.free();
console.log('Wasm smoke passed: archive I/O, budget slices, complete frame, CSS1 writes/backups, preferences, restart, quit, drop');
