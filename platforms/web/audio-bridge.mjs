// Worker-side transport: copies wasm views before transferring buffers to the page.
export class AudioBridge {
  constructor() { this.commands=[]; this.states=new Map(); this.time=0; }
  begin = (id,epoch,rate,channels,gain,fadeMs) => {
    this.states.set(id,{epoch,position:0});
    this.commands.push({op:'begin',id,epoch,rate,channels,gain,fadeMs});
  };
  samples = (id,epoch,view,ended) => {
    this.commands.push({op:'samples',id,epoch,pcm:view.slice(),ended});
  };
  stop = id => {this.states.delete(id);this.commands.push({op:'stop',id});};
  gain = (id,value) => this.commands.push({op:'gain',id,value});
  fade = (id,ms) => this.commands.push({op:'fade',id,ms});
  enabled = value => this.commands.push({op:'enabled',value});
  position = (id,epoch) => {const state=this.states.get(id);return state?.epoch===epoch?state.position:-1;};
  now = () => this.time;
  report({time,positions}) {
    if (!Number.isFinite(time) || time<this.time) return;
    this.time=time;
    for(const [id,epoch,position] of positions) {
      const state=this.states.get(id);
      if(state?.epoch===epoch && Number.isFinite(position)) state.position=position;
    }
  }
  take() {const commands=this.commands;this.commands=[];return commands;}
}
