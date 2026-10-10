import { expect, mock, test } from "bun:test";
import { renderHook } from "./hookHarness";
const react = await import("react");
mock.module("react", () => ({ ...react, useLayoutEffect: react.useEffect }));
(globalThis as any).window = { addEventListener() {}, removeEventListener() {} };
(globalThis as any).document = { addEventListener() {}, removeEventListener() {} };
const { EnginePicker } = await import("../../src/components/EnginePicker");
const nodes = (v: any): any[] => Array.isArray(v) ? v.flatMap(nodes) : v && typeof v === "object" ? [v,...nodes(v.props?.children)] : [];
const byTest = (v: any,id: string) => nodes(v).find(n=>n.props?.["data-testid"] === id);
const props: any = { activeEngine:"whisper", loadedEngine:"whisper", loadingTargetEngine:null,
  models:[{id:"tiny",display_name:"Tiny",size_mb:75}], currentModel:"tiny",
  graniteModels:[{id:"g",display_name:"Granite",size_mb:500}],currentGraniteModel:"g",
  qwen3Models:[{id:"q",display_name:"Qwen",size_mb:700}],currentQwen3Model:"q",
  downloadProgress:{},disabled:false,onSelectWhisperModel(){},onSelectGraniteModel(){},onSelectQwen3Model(){},onUnload(){},onOpenDownloads(){},onClose(){} };
test("remembered choices in unloaded engines have no loaded checkmark", async () => {
  const view=renderHook(()=>EnginePicker(props));
  byTest(view.handle.current,"ep-engine-row-granite").props.onClick(); await view.settle();
  expect(nodes(view.handle.current).some(n=>n.props?.className === "ep-model-check")).toBe(false);
  expect(byTest(view.handle.current,"ep-model-row-g").props["aria-checked"]).toBe(true);
});
test("only the resident model gets a check, and unloading or replacement loading removes it", async () => {
  let loaded:any="whisper",loading:any=null;
  const view=renderHook(()=>EnginePicker({...props,loadedEngine:loaded,loadingTargetEngine:loading}));
  byTest(view.handle.current,"ep-engine-row-whisper").props.onClick(); await view.settle();
  expect(nodes(view.handle.current).filter(n=>n.props?.className === "ep-model-check").length).toBe(1);
  expect(byTest(view.handle.current,"ep-model-row-tiny").props["aria-label"]).toContain("loaded");
  loaded=null;view.rerender();expect(nodes(view.handle.current).some(n=>n.props?.className === "ep-model-check")).toBe(false);
  loaded="whisper";loading="granite";view.rerender();expect(nodes(view.handle.current).some(n=>n.props?.className === "ep-model-check")).toBe(false);
});

test("native resident id wins over a remembered selection", async () => {
  const view=renderHook(()=>EnginePicker({...props,models:[...props.models,{id:"base",display_name:"Base",size_mb:150}],loadedModelId:"base"}));
  byTest(view.handle.current,"ep-engine-row-whisper").props.onClick(); await view.settle();
  expect(byTest(view.handle.current,"ep-model-row-base").props["aria-label"]).toContain("loaded");
  expect(byTest(view.handle.current,"ep-model-row-tiny").props["aria-label"]).not.toContain("loaded");
});
