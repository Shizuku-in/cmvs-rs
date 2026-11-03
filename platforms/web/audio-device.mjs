// Web Audio schedules a bounded stream of PCM buffers; its consumption clock drives VM queries.
export class AudioDevice {
  constructor(context) {
    this.context=context; this.channels=new Map();
    this.master=context.createGain(); this.master.connect(context.destination);
  }
  stop(id) {
    const channel=this.channels.get(id);
    if(!channel) return;
    this.channels.delete(id);
    for(const item of channel.items) {item.source.onended=null;item.source.stop();item.source.disconnect();}
    channel.base.disconnect();channel.envelope.disconnect();
  }
  envelopeAt(channel,time) {
    const fade=channel.fade;
    if(!fade || time>=fade.end) return fade?.to??1;
    if(time<=fade.start) return fade.from;
    return fade.from+(fade.to-fade.from)*(time-fade.start)/(fade.end-fade.start);
  }
  ramp(channel,from,to,start,end) {
    const param=channel.envelope.gain;
    param.cancelScheduledValues(start);param.setValueAtTime(from,start);
    if(end>start) param.linearRampToValueAtTime(to,end);else param.setValueAtTime(to,start);
    channel.fade={from,to,start,end};
  }
  apply(command) {
    const {id,op}=command, now=this.context.currentTime;
    if(op==='enabled') {this.master.gain.setValueAtTime(command.value?1:0,now);return;}
    if(op==='stop') {this.stop(id);return;}
    if(op==='begin') {
      this.stop(id);
      const base=this.context.createGain(), envelope=this.context.createGain();
      base.gain.setValueAtTime(command.gain,now);base.connect(envelope);envelope.connect(this.master);
      this.channels.set(id,{epoch:command.epoch,rate:command.rate,count:command.channels,base,envelope,items:[],frames:0,completed:0,next:now+.03,eof:false,ended:false,fadeMs:command.fadeMs,fade:null,release:null});return;
    }
    const channel=this.channels.get(id);
    if(!channel || channel.ended) return;
    if(op==='gain') {channel.base.gain.setValueAtTime(command.value,now);return;}
    if(op==='fade') {
      const end=now+command.ms/1000;
      this.ramp(channel,this.envelopeAt(channel,now),0,now,end);channel.release=end;return;
    }
    if(op!=='samples' || channel.epoch!==command.epoch) return;
    if(channel.eof) throw Error('PCM received after end of stream');
    const pcm=command.pcm;
    if(!(pcm instanceof Float32Array) || pcm.length%channel.count || pcm.some(x=>!Number.isFinite(x))) throw Error('Invalid audio PCM');
    if(channel.next-now>1) throw Error('Audio queue exceeds one second');
    if(pcm.length) {
      const frames=pcm.length/channel.count;
      const buffer=this.context.createBuffer(channel.count,frames,channel.rate);
      for(let c=0;c<channel.count;c++) {
        const plane=buffer.getChannelData(c);
        for(let i=0;i<frames;i++) plane[i]=pcm[i*channel.count+c];
      }
      const source=this.context.createBufferSource();source.buffer=buffer;source.connect(channel.base);
      const start=Math.max(channel.next,now+.02);
      if(channel.frames===0 && channel.fadeMs>0) this.ramp(channel,0,1,start,start+channel.fadeMs/1000);
      const item={source,start,frames,before:channel.frames,end:start+frames/channel.rate};
      channel.items.push(item);channel.frames+=frames;channel.next=item.end;
      source.onended=()=>{
        source.disconnect();channel.completed=Math.max(channel.completed,item.before+frames);
        const at=channel.items.indexOf(item);if(at>=0) channel.items.splice(at,1);
      };
      source.start(start);
    }
    channel.eof=command.ended;
  }
  clock() {
    const time=this.context.currentTime, positions=[];
    for(const [id,channel] of this.channels) {
      if(channel.release!==null && time>=channel.release && !channel.ended) {
        for(const item of channel.items) {item.source.onended=null;item.source.stop();item.source.disconnect();}
        channel.items=[];channel.ended=true;
      }
      let frames=channel.completed;
      for(const item of channel.items) {
        if(time>=item.start) frames=Math.max(frames,item.before+Math.min(item.frames,(time-item.start)*channel.rate));
      }
      const ended=channel.ended || (channel.eof && frames>=channel.frames);
      positions.push([id,channel.epoch,ended?-1:frames/channel.rate*1000]);
    }
    return {time:time*1000,positions};
  }
  close() {for(const id of [...this.channels.keys()]) this.stop(id);this.master.disconnect();return this.context.close();}
}
