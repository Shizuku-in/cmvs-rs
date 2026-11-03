import test from 'node:test';
import assert from 'node:assert/strict';
import {AudioBridge} from './audio-bridge.mjs';
import {AudioDevice} from './audio-device.mjs';
class Parameter {
  constructor(){this.values=[];}
  setValueAtTime(value,time){this.values.push([value,time]);}
  linearRampToValueAtTime(value,time){this.values.push([value,time]);}
  cancelScheduledValues(){}
}
class Context {
  currentTime=0; destination={}; sources=[];
  createGain(){return {gain:new Parameter(),connect(){},disconnect(){}};}
  createBuffer(count,frames,rate){const planes=Array.from({length:count},()=>new Float32Array(frames));return {duration:frames/rate,getChannelData:c=>planes[c]};}
  createBufferSource(){const source={connect(){},disconnect(){},start(time){this.time=time;},stop(){this.stopped=true;}};this.sources.push(source);return source;}
  close(){return Promise.resolve();}
}
const start={op:'begin',id:9,epoch:1,rate:8000,channels:1,gain:1,fadeMs:0};
const chunk={op:'samples',id:9,epoch:1,pcm:new Float32Array(800).fill(.25),ended:false};
test('transport copies wasm PCM views and ignores stale channel clocks',()=>{
  const bridge=new AudioBridge(), pcm=new Float32Array([.5]);
  bridge.begin(9,1,8000,1,1,0);bridge.samples(9,1,pcm,false);pcm[0]=0;
  assert.equal(bridge.take()[1].pcm[0],.5);
  bridge.begin(9,2,8000,1,1,0);
  bridge.report({time:100,positions:[[9,1,-1]]});assert.equal(bridge.position(9,2),0);
  bridge.report({time:125,positions:[[9,2,50]]});assert.equal(bridge.position(9,2),50);
  bridge.report({time:120,positions:[[9,2,-1]]});assert.equal(bridge.position(9,2),50);
});
test('consumption clock excludes initial scheduling delay and underrun silence',()=>{
  const context=new Context(),device=new AudioDevice(context);
  device.apply(start);device.apply(chunk);
  assert.equal(device.clock().positions[0][2],0);
  context.currentTime=.08;assert.ok(Math.abs(device.clock().positions[0][2]-50)<.001);
  context.currentTime=.4;assert.equal(device.clock().positions[0][2],100);
  device.apply({...chunk,ended:true});
  context.currentTime=.47;assert.ok(Math.abs(device.clock().positions[0][2]-150)<.001);
  context.currentTime=.6;assert.equal(device.clock().positions[0][2],-1);
});
test('queued buffers do not advance consumption or trigger unbounded refills',()=>{
  const context=new Context(),device=new AudioDevice(context),bridge=new AudioBridge();
  device.apply(start);bridge.begin(9,1,8000,1,1,0);
  let sent=0;
  for(let tick=0;tick<200;tick++) {
    context.currentTime=tick*.025;
    bridge.report(device.clock());
    const consumed=bridge.position(9,1);
    assert.ok(Math.abs(consumed-Math.max(0,context.currentTime-.03)*1000)<.001);
    while(sent-consumed<250) {device.apply(chunk);sent+=100;}
    assert.ok(sent-consumed<=350);
    assert.ok(Math.abs(device.clock().positions[0][2]-consumed)<.001);
    const last=context.sources.at(-1);
    assert.ok(last.time+last.buffer.duration-context.currentTime<.4);
  }
});
test('replacing or stopping a channel cancels queued sources and ignores stale PCM',()=>{
  const context=new Context(),device=new AudioDevice(context);
  device.apply(start);device.apply(chunk);const old=context.sources[0];
  device.apply({...start,epoch:2});assert.equal(old.stopped,true);
  device.apply(chunk);assert.equal(context.sources.length,1);
  device.apply({...chunk,epoch:2});device.apply({op:'stop',id:9});
  assert.equal(context.sources[1].stopped,true);assert.deepEqual(device.clock().positions,[]);
});
test('mute keeps consuming while fade-out cancels all remaining queued audio',()=>{
  const context=new Context(),device=new AudioDevice(context);
  device.apply({...start,fadeMs:100});device.apply(chunk);
  device.apply({op:'enabled',value:false});context.currentTime=.08;
  assert.ok(device.clock().positions[0][2]>0);assert.equal(device.master.gain.values.at(-1)[0],0);
  device.apply({op:'fade',id:9,ms:20});context.currentTime=.11;
  assert.equal(device.clock().positions[0][2],-1);assert.equal(context.sources[0].stopped,true);
});
