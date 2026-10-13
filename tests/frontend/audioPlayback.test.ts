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

test("hiding the app stops playback and blocks stale media-key resume until explicitly played again", async () => {
 const { setAudioPlaybackVisible } = await import("../../src/utils/audioPlayback");
 const audio = { pauses:0, onpause:null as any, onplay:null as any, pause(){this.pauses++;this.onpause?.({});} };
 let interrupted=0;
 setAudioPlaybackVisible(true);
 claimAudioPlayback(audio as any,()=>interrupted++);
 setAudioPlaybackVisible(false);
 expect(audio.pauses).toBe(1);
 expect(interrupted).toBe(1);
 audio.onplay({});
 expect(audio.pauses).toBe(2);
 setAudioPlaybackVisible(true);
 audio.onplay({});
 expect(audio.pauses).toBe(3);
 claimAudioPlayback(audio as any);
 audio.onplay({});
 expect(audio.pauses).toBe(3);
 audio.pause();
});

test("headset play resumes the selected visible player but is ignored after close to tray", async () => {
 const { installAudioPlaybackLifecycle, setAudioPlaybackVisible } = await import("../../src/utils/audioPlayback");
 const oldDocument=globalThis.document, oldWindow=globalThis.window;
 const navDescriptor=Object.getOwnPropertyDescriptor(globalThis,"navigator");
 const handlers=new Map<string, (()=>void)|null>();
 const session={playbackState:"none",metadata:null,setActionHandler:(name:string,handler:(()=>void)|null)=>handlers.set(name,handler)};
 (globalThis as any).document={visibilityState:"visible",addEventListener(){},removeEventListener(){}};
 (globalThis as any).window={addEventListener(){},removeEventListener(){}};
 Object.defineProperty(globalThis,"navigator",{configurable:true,value:{mediaSession:session}});
 try {
  const cleanup=installAudioPlaybackLifecycle();
  const audio={plays:0,onpause:null as any,onplay:null as any,pause(){this.onpause?.({});},async play(){this.plays++;this.onplay?.({});}};
  claimAudioPlayback(audio as any);
  audio.pause();
  handlers.get("play")!();
  expect(audio.plays).toBe(1);
  setAudioPlaybackVisible(false);
  handlers.get("play")!();
  expect(audio.plays).toBe(1);
  setAudioPlaybackVisible(true);
  handlers.get("play")!();
  expect(audio.plays).toBe(1);
  cleanup();
  expect(handlers.get("play")).toBeNull();
 } finally {
  (globalThis as any).document=oldDocument;(globalThis as any).window=oldWindow;
  if(navDescriptor)Object.defineProperty(globalThis,"navigator",navDescriptor);else delete (globalThis as any).navigator;
  setAudioPlaybackVisible(true);
 }
});

test("a hidden page (minimised or covered window) keeps playing; becoming visible re-allows after tray close", async () => {
 const { installAudioPlaybackLifecycle, setAudioPlaybackVisible } = await import("../../src/utils/audioPlayback");
 const oldDocument=globalThis.document, oldWindow=globalThis.window;
 const navDescriptor=Object.getOwnPropertyDescriptor(globalThis,"navigator");
 const listeners=new Map<string,()=>void>();
 const doc={visibilityState:"visible",addEventListener(name:string,fn:()=>void){listeners.set(name,fn);},removeEventListener(){}};
 (globalThis as any).document=doc;
 (globalThis as any).window={addEventListener(name:string,fn:()=>void){listeners.set("window:"+name,fn);},removeEventListener(){}};
 Object.defineProperty(globalThis,"navigator",{configurable:true,value:{}});
 try {
  setAudioPlaybackVisible(true);
  const cleanup=installAudioPlaybackLifecycle();
  const audio={pauses:0,onpause:null as any,onplay:null as any,pause(){this.pauses++;this.onpause?.({});}};
  claimAudioPlayback(audio as any);
  doc.visibilityState="hidden";
  listeners.get("visibilitychange")!();
  expect(audio.pauses).toBe(0);
  setAudioPlaybackVisible(false); // closed to tray
  expect(audio.pauses).toBe(1);
  doc.visibilityState="visible";
  listeners.get("visibilitychange")!();
  const again={pauses:0,onpause:null as any,onplay:null as any,pause(){this.pauses++;this.onpause?.({});}};
  claimAudioPlayback(again as any);
  expect(again.pauses).toBe(0);
  cleanup();
 } finally {
  (globalThis as any).document=oldDocument;(globalThis as any).window=oldWindow;
  if(navDescriptor)Object.defineProperty(globalThis,"navigator",navDescriptor);else delete (globalThis as any).navigator;
  setAudioPlaybackVisible(true);
 }
});
