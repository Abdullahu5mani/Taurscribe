import { expect, test } from "bun:test";
import { claimAudioPlayback } from "../../src/utils/audioPlayback";
test("starting another clip pauses its predecessor across surfaces; stale pause cannot release new player",()=>{
 const interrupted:string[]=[];
 const make=()=>({pauses:0,onpause:null as any,pause(){this.pauses++;this.onpause?.({});}});
 const a=make(),b=make(),c=make();
 claimAudioPlayback(a as any,()=>interrupted.push("vault"));
 claimAudioPlayback(b as any,()=>interrupted.push("meeting"));
 expect(a.pauses).toBe(1);expect(interrupted).toEqual(["vault"]);
 a.onpause({});claimAudioPlayback(c as any);
 expect(b.pauses).toBe(1);expect(interrupted).toEqual(["vault","meeting"]);
 claimAudioPlayback(c as any);expect(c.pauses).toBe(0);
 c.pause();claimAudioPlayback(a as any);expect(c.pauses).toBe(1);
 a.pause();
});
