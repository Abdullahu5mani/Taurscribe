import { expect, mock, test } from 'bun:test';
import { renderHook } from './hookHarness';
const calls:{cmd:string,args:any}[]=[];
let finish:((path:string)=>void)|null=null;
let delayed=false;
const realCore={...(await import('@tauri-apps/api/core'))};
mock.module('@tauri-apps/api/core',()=>({...realCore,convertFileSrc:(path:string)=>path,invoke:async(cmd:string,args:any)=>{
 calls.push({cmd,args});
 if(cmd==='voice_separation_available')return true;
 if(cmd==='generate_meeting_speaker_preview'){if(delayed)return new Promise<string>(resolve=>finish=resolve);return '/owned/new.wav';}
 if(cmd==='separate_meeting_sample')return ['/owned/isolated-a.wav','/owned/isolated-b.wav'];
 return null;
}}));
const {SpeakerPreviewModal}=await import('../../src/components/SpeakerPreviewModal');
const nodes=(root:any):any[]=>root&&typeof root==='object'?(Array.isArray(root)?root.flatMap(nodes):[root,...nodes(root.props?.children)]):[];
const text=(root:any):string=>root==null?'':typeof root!=='object'?String(root):Array.isArray(root)?root.map(text).join(''):text(root.props?.children);
const render=()=>renderHook(()=>SpeakerPreviewModal({meetingId:9,speakerId:'speaker2',speakerName:'Sam',initialPath:'/retained/clip.wav',onClose:()=>{}}));
test('comparison keeps original and new previews; closing deletes only generated sources',async()=>{
 calls.length=0;delayed=false;
 const view=render();await view.settle();
 await nodes(view.handle.current).find(n=>n.type==='button'&&text(n)==='Generate another sample').props.onClick();await view.settle();
 expect(nodes(view.handle.current).filter(n=>n.type==='audio').map(n=>n.props.src)).toEqual(['/retained/clip.wav','/owned/new.wav']);
 expect(calls.find(c=>c.cmd==='generate_meeting_speaker_preview')?.args.excludedPaths).toEqual(['/retained/clip.wav']);
 await nodes(view.handle.current).filter(n=>n.type==='button'&&text(n)==='Isolate overlapping voices')[1].props.onClick();await view.settle();
 expect(nodes(view.handle.current).filter(n=>n.type==='audio')).toHaveLength(4);
 view.unmount();
 expect(calls.filter(c=>c.cmd==='discard_generated_voice_sample').map(c=>c.args.path).sort()).toEqual(['/owned/isolated-a.wav','/owned/isolated-b.wav','/owned/new.wav']);
});
test('preview completed after close is discarded instead of leaking or enrolling',async()=>{
 calls.length=0;delayed=true;finish=null;
 const view=render();await view.settle();
 const pending=nodes(view.handle.current).find(n=>n.type==='button'&&text(n)==='Generate another sample').props.onClick();
 view.unmount();finish!('/owned/late.wav');await pending;
 expect(calls.some(c=>c.cmd==='discard_generated_voice_sample'&&c.args.path==='/owned/late.wav')).toBe(true);
 expect(calls.some(c=>c.cmd==='add_vault_voice_sample')).toBe(false);
});
