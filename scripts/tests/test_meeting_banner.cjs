// Run with: node scripts/tests/test_meeting_banner.cjs
// Exercise the real component's effects without a webview or native recording.
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const path = require('node:path');
const ts = require('typescript');

function transpile(relativePath) {
  const source = fs.readFileSync(path.join(__dirname, '../../src', relativePath), 'utf8');
  return ts.transpileModule(source, {
    compilerOptions: { module: ts.ModuleKind.CommonJS, jsx: ts.JsxEmit.ReactJSX },
  }).outputText;
}
const compiled = transpile('components/MeetingBanner.tsx');
const autoRecordCompiled = transpile('utils/autoRecord.ts');

async function scenario(action, delay, expectedStarts) {
  const states = [], refs = [], effects = [], pending = [];
  const timers = new Map();
  let si = 0, ri = 0, ei = 0, starts = 0, timerId = 0, resolveEnabled;
  const enabled = new Promise(resolve => { resolveEnabled = resolve; });
  const react = {
    useState(initial) {
      const index = si++;
      if (!(index in states)) states[index] = initial;
      return [states[index], value => {
        states[index] = typeof value === 'function' ? value(states[index]) : value;
      }];
    },
    useRef(initial) { return refs[ri++] ??= { current: initial }; },
    useEffect(fn, deps) {
      const index = ei++, previous = effects[index];
      if (!previous || deps.some((value, i) => value !== previous.deps[i])) {
        pending.push(() => {
          previous?.cleanup?.();
          effects[index] = { deps, cleanup: fn() };
        });
      }
    },
  };
  const jsx = (type, props) => ({ type, props });
  const context = {
    exports: {},
    require(name) {
      if (name === 'react') return react;
      if (name === 'react/jsx-runtime') return { jsx, jsxs: jsx };
      if (name === '@tauri-apps/api/core') return { invoke: () => enabled };
      if (name === '@tauri-apps/plugin-store') {
        return { Store: { load: async () => ({ get: async key => key === 'delay' ? delay : true }) } };
      }
      if (name === './settings/types') {
        return { MEETING_KEYS: { showBanner: 'banner', autoRecordDelay: 'delay' }, DEFAULT_AUTORECORD_DELAY: 5 };
      }
      if (name === '../utils/autoRecord') {
        // Share this scenario's fake timers with the helper module.
        const module = { exports: {} };
        vm.runInNewContext(autoRecordCompiled, {
          exports: module.exports, setInterval: context.setInterval, clearInterval: context.clearInterval,
        });
        return module.exports;
      }
      if (name.endsWith('.css')) return {};
      throw Error(name);
    },
    setInterval(fn) { timers.set(++timerId, fn); return timerId; },
    clearInterval(id) { timers.delete(id); },
  };
  vm.runInNewContext(compiled, context);
  const meeting = { pid: 123, title: 'Test call', platform: 'Zoom' };
  function render(currentMeeting) {
    si = ri = ei = 0;
    const tree = context.exports.MeetingBanner({
      meeting: currentMeeting, isRecording: false, onStartDualRecording: () => starts++,
    });
    pending.splice(0).forEach(fn => fn());
    return tree;
  }
  function find(node, id) {
    if (!node) return;
    if (node.props?.id === id) return node;
    for (const child of [node.props?.children].flat()) {
      const match = find(child, id);
      if (match) return match;
    }
  }
  const tree = render(meeting);
  if (action === 'ended') render(null);
  if (action === 'unmounted') effects.forEach(effect => effect.cleanup?.());
  if (action === 'dismissed') {
    find(tree, 'meeting-banner-dismiss-btn').props.onClick();
    render(meeting);
  }
  if (action === 'manual-start') find(tree, 'meeting-banner-record-btn').props.onClick();
  resolveEnabled(true);
  await new Promise(resolve => setImmediate(resolve));
  for (let i = 0; i < delay + 1; i++) [...timers.values()].forEach(fn => fn());
  assert.equal(starts, expectedStarts, `${action}, delay=${delay}`);
  effects.forEach(effect => effect.cleanup?.());
}

(async () => {
  for (const delay of [0, 3]) {
    for (const action of ['ended', 'dismissed', 'unmounted']) await scenario(action, delay, 0);
    await scenario('manual-start', delay, 1);
    await scenario('still-active', delay, 1);
  }
  console.log('Meeting banner: 10 cancellation and auto-record checks passed');
})().catch(error => { console.error(error); process.exitCode = 1; });
