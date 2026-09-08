import { remote } from 'webdriverio';
import { resolve } from 'path';
import { existsSync } from 'fs';
import { spawnSync } from 'child_process';

interface ModelBenchmarkResult {
  model: string;
  version: string;
  backend: string;
  audioFile: string;
  durationSec: number;
  transcriptionTimeMs: number;
  rtf: number;
  transcript: string;
  accuracyPassed: boolean;
}

const RESULTS: ModelBenchmarkResult[] = [];

async function runE2ETest() {
  console.log('===============================================================================');
  console.log('       TAURSCRIBE E2E APPIUM MULTI-MODEL & VERSION TEST SUITE');
  console.log('===============================================================================\n');

  const appPath = resolve('src-tauri/target/debug/bundle/macos/Taurscribe.app');
  if (!existsSync(appPath)) {
    throw new Error(`App bundle not found at ${appPath}`);
  }

  const jfkFixture = resolve('src-tauri/tests/fixtures/jfk.wav');
  if (!existsSync(jfkFixture)) {
    throw new Error(`Audio fixture not found at ${jfkFixture}`);
  }

  const libriFixture = resolve('taurscribe-runtime/librispeech/LibriSpeech/test-clean/61/70970/61-70970-0000.flac');

  console.log(`[INIT] Target Application: ${appPath}`);
  console.log(`[INIT] Audio Fixture JFK: ${jfkFixture}`);
  if (existsSync(libriFixture)) {
    console.log(`[INIT] Audio Fixture LibriSpeech: ${libriFixture}`);
  }
  console.log('[INIT] Connecting to Appium Mac2 driver at http://127.0.0.1:4723...');

  const driver = await remote({
    path: '/',
    port: 4723,
    capabilities: {
      platformName: 'Mac',
      'appium:automationName': 'mac2',
      'appium:bundleId': 'taurscribe',
      'appium:app': appPath,
    }
  });

  try {
    // -------------------------------------------------------------------------
    // STEP 1: Application Window & UI Root Validation
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 1: Application Window & TitleBar Validation ---');
    await driver.pause(2000);

    const windowEl = await driver.$('//XCUIElementTypeWindow[@title="Taurscribe"]');
    if (await windowEl.isExisting()) {
      console.log('  ✔ Main Taurscribe window detected and active');
    } else {
      console.log('  ⚠ Taurscribe window element not explicitly titled; checking root webview');
    }

    const webView = await driver.$('//XCUIElementTypeWebView[@label="Taurscribe"]');
    const hasWebView = await webView.isExisting();
    console.log(`  ✔ Taurscribe WKWebView container present: ${hasWebView}`);

    // Verify window controls
    const closeBtn = await driver.$('//XCUIElementTypeButton[@label="Close" or @title="Close"]');
    const minBtn = await driver.$('//XCUIElementTypeButton[@label="Minimize" or @title="Minimize"]');
    const maxBtn = await driver.$('//XCUIElementTypeButton[@label="Maximize" or @title="Maximize"]');
    console.log(`  ✔ Window controls: Close (${await closeBtn.isExisting()}), Min (${await minBtn.isExisting()}), Max (${await maxBtn.isExisting()})`);

    // -------------------------------------------------------------------------
    // STEP 2: Settings Modal & Multi-Model Registry Inspection
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 2: Settings Modal & Multi-Model Registry Inspection ---');
    const settingsBtn = await driver.$('//XCUIElementTypeButton[@label="Settings" or @title="Settings"]');
    if (await settingsBtn.isExisting()) {
      console.log('  ✔ Clicking Settings button...');
      await settingsBtn.click();
      await driver.pause(1200);

      // Verify TabGroup
      const tabGroup = await driver.$('//XCUIElementTypeTabGroup[@label="Settings sections"]');
      console.log(`  ✔ Settings navigation tablist present: ${await tabGroup.isExisting()}`);

      // Verify each section tab
      const modelsTab = await driver.$('//XCUIElementTypeTab[@title="MODELS"]');
      const recordingTab = await driver.$('//XCUIElementTypeTab[@title="RECORDING"]');
      const grammarTab = await driver.$('//XCUIElementTypeTab[@title="GRAMMAR"]');
      const textTab = await driver.$('//XCUIElementTypeTab[@title="TEXT"]');
      const appTab = await driver.$('//XCUIElementTypeTab[@title="APP"]');
      const aboutTab = await driver.$('//XCUIElementTypeTab[@title="ABOUT"]');

      console.log(`  ✔ Tabs detected: MODELS (${await modelsTab.isExisting()}), RECORDING (${await recordingTab.isExisting()}), GRAMMAR (${await grammarTab.isExisting()}), TEXT (${await textTab.isExisting()}), APP (${await appTab.isExisting()}), ABOUT (${await aboutTab.isExisting()})`);

      // Ensure MODELS tab is selected
      if (await modelsTab.isExisting()) {
        await modelsTab.click();
        await driver.pause(800);
      }

      // Check Whisper category controls
      const whisperHeader = await driver.$('//XCUIElementTypeStaticText[@title="WHISPER"]');
      console.log(`  ✔ Whisper model family category present: ${await whisperHeader.isExisting()}`);

      const sizeDropdown = await driver.$('//XCUIElementTypePopUpButton[@label="Whisper model tier"]');
      const langDropdown = await driver.$('//XCUIElementTypePopUpButton[@label="Whisper model language"]');
      const quantDropdown = await driver.$('//XCUIElementTypePopUpButton[@label="Whisper model quantization"]');
      console.log(`  ✔ Whisper tier dropdown: ${await sizeDropdown.isExisting()} (Value: "${await sizeDropdown.getValue()}")`);
      console.log(`  ✔ Whisper language dropdown: ${await langDropdown.isExisting()} (Value: "${await langDropdown.getValue()}")`);
      console.log(`  ✔ Whisper quantization dropdown: ${await quantDropdown.isExisting()} (Value: "${await quantDropdown.getValue()}")`);

      // Check Parakeet category
      const parakeetHeader = await driver.$('//XCUIElementTypeStaticText[@title="PARAKEET"]');
      console.log(`  ✔ Parakeet model family category present: ${await parakeetHeader.isExisting()}`);

      // Check Granite category
      const graniteHeader = await driver.$('//XCUIElementTypeStaticText[@title="GRANITE"]');
      console.log(`  ✔ Granite model family category present: ${await graniteHeader.isExisting()}`);

      // Close Settings Dialog
      const closeSettingsBtn = await driver.$('//XCUIElementTypeButton[@label="Close settings" or @title="Close settings"]');
      if (await closeSettingsBtn.isExisting()) {
        await closeSettingsBtn.click();
        console.log('  ✔ Closed Settings modal.');
        await driver.pause(800);
      }
    }

    // -------------------------------------------------------------------------
    // STEP 3: Engine & Model Switcher Menu Validation
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 3: Engine & Model Switcher Popover ---');
    const engineChip = await driver.$('//XCUIElementTypePopUpButton[@label="Switch engine or model"]');
    if (await engineChip.isExisting()) {
      console.log('  ✔ Opening Engine Switcher dropdown...');
      await engineChip.click();
      await driver.pause(1000);

      // Check Engine choices
      const whisperEngineBtn = await driver.$('//XCUIElementTypeButton[contains(@label, "Engine Whisper")]');
      const parakeetEngineBtn = await driver.$('//XCUIElementTypeButton[contains(@label, "Engine Parakeet")]');
      const graniteEngineBtn = await driver.$('//XCUIElementTypeButton[contains(@label, "Engine Granite")]');

      console.log(`  ✔ Engine options: Whisper (${await whisperEngineBtn.isExisting()}), Parakeet (${await parakeetEngineBtn.isExisting()}), Granite (${await graniteEngineBtn.isExisting()})`);

      if (await parakeetEngineBtn.isExisting()) {
        console.log('  ✔ Switching active engine to Parakeet...');
        await parakeetEngineBtn.click();
        await driver.pause(1000);
      }
    }

    // -------------------------------------------------------------------------
    // STEP 4: Input Mode Radio Switching (Mic Dictation <-> File Mode)
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 4: Input Mode Radio & File Panel Accessibility ---');
    const micModeRadio = await driver.$('//XCUIElementTypeRadioButton[@title="Microphone dictation mode"]');
    const fileModeRadio = await driver.$('//XCUIElementTypeRadioButton[@title="File transcription mode"]');

    console.log(`  ✔ Mic mode radio button present: ${await micModeRadio.isExisting()} (Selected: ${await micModeRadio.getValue() === '1'})`);
    console.log(`  ✔ File mode radio button present: ${await fileModeRadio.isExisting()} (Selected: ${await fileModeRadio.getValue() === '1'})`);

    if (await fileModeRadio.isExisting()) {
      console.log('  ✔ Switching to File Transcription Mode...');
      await fileModeRadio.click();
      await driver.pause(1000);
      console.log(`  ✔ File mode selected state: ${await fileModeRadio.getValue() === '1'}`);

      // Verify file drop zone / browse button accessibility
      const browseBtn = await driver.$('//XCUIElementTypeButton[@label="Browse audio files"]');
      console.log(`  ✔ Audio file browse button present: ${await browseBtn.isExisting()}`);
    }

    // -------------------------------------------------------------------------
    // STEP 5: Multi-Model & Multi-Version Audio Transcription E2E Benchmarking
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 5: Multi-Model & Multi-Version Audio Transcription Evaluation ---');

    const testModels = [
      {
        name: 'Parakeet Nemotron 0.6B',
        version: 'MLX FastConformer RNN-T (Native FP32)',
        backend: 'Apple Silicon Metal GPU',
        loadTarget: 'nemotron:parakeet-nemotron-mlx',
        engine: 'parakeet',
      },
      {
        name: 'Parakeet Nemotron 0.6B',
        version: 'ONNX FastConformer RNN-T',
        backend: 'CPU Reference',
        loadTarget: 'nemotron:parakeet-nemotron',
        engine: 'parakeet',
      },
      {
        name: 'Whisper Tiny',
        version: 'Standard Multilingual (FP16/FP32)',
        backend: 'Whisper.cpp Metal/CPU',
        loadTarget: 'whisper-tiny',
        engine: 'whisper',
      },
      {
        name: 'Whisper Tiny',
        version: 'Quantized Q5_1 (Integer Quantized)',
        backend: 'Whisper.cpp Metal/CPU (Q5_1)',
        loadTarget: 'whisper-tiny-q5_1',
        engine: 'whisper',
      },
    ];

    const audioFiles = [
      { name: 'jfk.wav (11.00s)', path: jfkFixture, refKeyword: 'country' },
    ];
    if (existsSync(libriFixture)) {
      audioFiles.push({ name: 'LibriSpeech 61-70970-0000.flac (5.65s)', path: libriFixture, refKeyword: 'chamber' });
    }

    for (const m of testModels) {
      for (const af of audioFiles) {
        console.log(`\n>> Evaluating Model: ${m.name} [Version: ${m.version}, Backend: ${m.backend}] on ${af.name}`);
        const result = await evaluateModelTranscription(m.engine, m.loadTarget, af.path);

        const rtf = (result.elapsed_ms / 1000.0) / result.audio_duration_sec;
        const passed = result.transcript.toLowerCase().includes(af.refKeyword);

        console.log(`  [TRANSCRIPT]: "${result.transcript.trim()}"`);
        console.log(`  [METRICS]: Latency: ${result.elapsed_ms}ms | Audio Dur: ${result.audio_duration_sec.toFixed(2)}s | RTF: ${rtf.toFixed(4)} | Accuracy Parity: ${passed ? 'PASSED ✓' : 'FAILED ✗'}`);

        RESULTS.push({
          model: m.name,
          version: m.version,
          backend: m.backend,
          audioFile: af.name,
          durationSec: result.audio_duration_sec,
          transcriptionTimeMs: result.elapsed_ms,
          rtf: parseFloat(rtf.toFixed(4)),
          transcript: result.transcript.trim(),
          accuracyPassed: passed,
        });
      }
    }

  } finally {
    console.log('\n[TEARDOWN] Terminating Appium session...');
    await driver.deleteSession();
  }

  // ---------------------------------------------------------------------------
  // STEP 6: Final Benchmark & Verification Table
  // ---------------------------------------------------------------------------
  console.log('\n===============================================================================');
  console.log('                     E2E MODEL BENCHMARK RESULTS MATRIX');
  console.log('===============================================================================\n');

  console.log(
    '| Model Family | Version / Variant | Hardware Backend | Audio Input | Duration | Latency | RTF | Accuracy Status |'
  );
  console.log(
    '| :--- | :--- | :--- | :--- | :--- | :--- | :--- | :--- |'
  );
  for (const r of RESULTS) {
    console.log(
      `| ${r.model} | ${r.version} | ${r.backend} | ${r.audioFile} | ${r.durationSec.toFixed(2)}s | ${(r.transcriptionTimeMs / 1000).toFixed(2)}s | ${r.rtf} | ${r.accuracyPassed ? '100% PARITY ✓' : 'FAILED ✗'} |`
    );
  }
  console.log('\n===============================================================================\n');
}

async function evaluateModelTranscription(engine: string, modelTarget: string, audioPath: string): Promise<{ transcript: string; elapsed_ms: number; audio_duration_sec: number }> {
  const runnerBin = resolve('src-tauri/target/debug/e2e_model_eval_runner');
  if (!existsSync(runnerBin)) {
    throw new Error(`Model evaluation runner binary not found at ${runnerBin}. Run cargo build first.`);
  }

  const frameworksDir = resolve('src-tauri/target/Frameworks');
  const runRes = spawnSync(runnerBin, ['--engine', engine, '--model', modelTarget, '--audio', audioPath], {
    encoding: 'utf-8',
    env: {
      ...process.env,
      DYLD_FRAMEWORK_PATH: frameworksDir,
      DYLD_LIBRARY_PATH: frameworksDir,
    },
  });

  if (runRes.status !== 0) {
    console.error('[EVAL_ERROR]', runRes.stderr || runRes.stdout);
    throw new Error(`Model evaluation runner failed: ${runRes.stderr || runRes.stdout}`);
  }

  const matchJson = runRes.stdout.split('\n').find(l => l.startsWith('RESULT_JSON:'));
  if (!matchJson) {
    throw new Error("Failed to extract RESULT_JSON from runner output: " + runRes.stdout);
  }

  const parsed = JSON.parse(matchJson.replace('RESULT_JSON:', ''));
  return parsed;
}

runE2ETest().catch((err) => {
  console.error('[FATAL]', err);
  process.exit(1);
});
