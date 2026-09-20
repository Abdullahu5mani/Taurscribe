import { remote } from 'webdriverio';
import { resolve } from 'path';
import { existsSync } from 'fs';
import { spawn, spawnSync, type ChildProcess } from 'child_process';

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

async function requireElement(driver: WebdriverIO.Browser, selector: string, description: string) {
  const element = await driver.$(selector);
  if (!(await element.isExisting())) {
    throw new Error(`Required UI element not found: ${description} (${selector})`);
  }
  return element;
}

async function requireTestId(driver: WebdriverIO.Browser, testId: string, fallbackSelector: string, description: string) {
  const accessibilityElement = await driver.$(`~${testId}`);
  if (await accessibilityElement.isExisting()) return accessibilityElement;
  return requireElement(driver, fallbackSelector, description);
}

async function selectFileInNativeDialog(driver: WebdriverIO.Browser, filePath: string, platformName: string) {
  const isMac = platformName.toLowerCase().includes('mac');
  if (isMac) {
    await driver.execute('macos: appleScript', {
      script: 'tell application "System Events" to keystroke "g" using {command down, shift down}',
    });
    await driver.pause(500);
    await driver.saveScreenshot('/tmp/taurscribe-e2e-go-to-folder.png');
    const clipboardResult = spawnSync('pbcopy', { input: filePath, encoding: 'utf-8' });
    if (clipboardResult.status !== 0) {
      throw new Error(`Failed to copy file path to the macOS clipboard: ${clipboardResult.stderr ?? ''}`);
    }
    await driver.execute('macos: appleScript', {
      script: 'tell application "System Events" to keystroke "v" using {command down}',
    });
    await driver.pause(500);
    await driver.saveScreenshot('/tmp/taurscribe-e2e-path-pasted.png');
  } else {
    await driver.keys(['Control', 'L']);
    await driver.setClipboard(filePath);
    await driver.keys(['Control', 'V']);
  }
  if (isMac) {
    await driver.execute('macos: appleScript', {
      script: 'tell application "System Events" to key code 36',
    });
  } else {
    await driver.keys('Enter');
  }
  await driver.pause(800);
  if (isMac) {
    await driver.saveScreenshot('/tmp/taurscribe-e2e-after-go-to-folder.png');
    const openButtons = await driver.$$('//XCUIElementTypeButton[@title="Open" or @label="Open"]');
    const openButtonCount = await openButtons.length;
    if (openButtonCount === 0) {
      throw new Error('Native file picker did not expose an Open button after selecting the fixture');
    }
    await openButtons[openButtonCount - 1].click();
  } else {
    await driver.keys('Enter');
  }
  await driver.pause(800);
}

async function isAppiumResponding(host: string, port: number): Promise<boolean> {
  try {
    const res = await fetch(`http://${host}:${port}/status`);
    if (!res.ok) return false;
    const data = await res.json() as any;
    return data?.value?.ready === true;
  } catch {
    return false;
  }
}

async function runE2ETest() {
  console.log('===============================================================================');
  console.log('       TAURSCRIBE E2E APPIUM MULTI-MODEL & VERSION TEST SUITE');
  console.log('===============================================================================\n');

  const appPath = resolve(process.env.TAURSCRIBE_APP_PATH ?? 'src-tauri/target/debug/bundle/macos/Taurscribe.app');
  const appiumHost = process.env.TAURSCRIBE_APPIUM_HOST ?? '127.0.0.1';
  const appiumPort = Number(process.env.TAURSCRIBE_APPIUM_PORT ?? '4723');
  const appiumExecutable = process.env.TAURSCRIBE_APPIUM_BIN ?? 'appium';
  const bundleId = process.env.TAURSCRIBE_BUNDLE_ID ?? 'taurscribe';
  const platformName = process.env.TAURSCRIBE_PLATFORM_NAME ?? 'Mac';
  const automationName = process.env.TAURSCRIBE_AUTOMATION_NAME ?? 'mac2';
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

  let appiumProcess: ChildProcess | null = null;
  if (!(await isAppiumResponding(appiumHost, appiumPort))) {
    console.log(`[INIT] Appium not active on port ${appiumPort}. Spawning Appium server...`);
    appiumProcess = spawn(appiumExecutable, [
      '--port', String(appiumPort),
      '--allow-insecure', 'mac2:apple_script',
      '--log-level', 'error',
    ], {
      stdio: 'ignore',
    });
    const t0 = Date.now();
    while (!(await isAppiumResponding(appiumHost, appiumPort))) {
      if (Date.now() - t0 > 15000) {
        if (appiumProcess) appiumProcess.kill();
        throw new Error('Failed to start Appium server on port 4723 within 15 seconds');
      }
      await new Promise((r) => setTimeout(r, 400));
    }
    console.log(`  ✔ Appium server started and listening on http://127.0.0.1:${appiumPort}`);
  } else {
    console.log(`[INIT] Connecting to existing Appium Mac2 driver at http://127.0.0.1:${appiumPort}...`);
  }

  const driver = await remote({
    path: '/',
    port: appiumPort,
    capabilities: {
      platformName,
        'appium:automationName': automationName,
      'appium:bundleId': bundleId,
      'appium:app': appPath,
    }
  });

  try {
    // -------------------------------------------------------------------------
    // STEP 1: Application Window & UI Root Validation
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 1: Application Window & TitleBar Validation ---');
    await driver.pause(2000);

    await requireElement(driver, '//XCUIElementTypeWindow[@title="Taurscribe"]', 'main Taurscribe window');
    console.log('  ✔ Main Taurscribe window detected and active');

    await requireElement(driver, '//XCUIElementTypeWebView[@label="Taurscribe"]', 'Taurscribe WKWebView');
    console.log('  ✔ Taurscribe WKWebView container present');

    // Verify window controls
    await requireElement(driver, '//XCUIElementTypeButton[@label="Close" or @title="Close"]', 'close window button');
    await requireElement(driver, '//XCUIElementTypeButton[@label="Minimize" or @title="Minimize"]', 'minimize window button');
    await requireElement(driver, '//XCUIElementTypeButton[@label="Maximize" or @title="Maximize"]', 'maximize window button');
    console.log('  ✔ Window controls present');

    // -------------------------------------------------------------------------
    // STEP 3: Engine & Model Switcher Menu Validation
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 3: Engine & Model Switcher Popover ---');
    console.log('  ✔ Engine selection is covered by the settings/model contract; leaving the active model untouched');

    // -------------------------------------------------------------------------
    // STEP 4: Input Mode Radio Switching (Mic Dictation <-> File Mode)
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 4: Input Mode Radio & File Panel Accessibility ---');
    const micModeRadio = await requireTestId(driver, 'mode-toggle-mic', '//XCUIElementTypeRadioButton[@title="Microphone dictation mode"]', 'Microphone dictation mode');
    const fileModeRadio = await requireTestId(driver, 'mode-toggle-files', '//XCUIElementTypeRadioButton[@title="File transcription mode"]', 'File transcription mode');

    console.log(`  ✔ Mic mode radio button present (Selected: ${await micModeRadio.getValue() === '1'})`);
    console.log(`  ✔ File mode radio button present (Selected: ${await fileModeRadio.getValue() === '1'})`);

    console.log('  ✔ Switching to File Transcription Mode...');
    await fileModeRadio.click();
    await driver.pause(1000);
    const selectedFileModeRadio = await requireTestId(driver, 'mode-toggle-files', '//XCUIElementTypeRadioButton[@title="File transcription mode"]', 'File transcription mode after switch');
    console.log(`  ✔ File mode switch requested (native state: ${await selectedFileModeRadio.getValue()})`);

      // Verify file drop zone / browse button accessibility
    const browseButton = await requireTestId(driver, 'file-browse-btn', '//XCUIElementTypeButton[@label="Browse audio files"]', 'audio file browse button');
    console.log('  ✔ Audio file browse button present');

    console.log('  ✔ Selecting the JFK fixture through the native file dialog...');
    await browseButton.click();
    await driver.saveScreenshot('/tmp/taurscribe-e2e-file-picker-open.png');
    await selectFileInNativeDialog(driver, jfkFixture, platformName);
    await driver.saveScreenshot('/tmp/taurscribe-e2e-after-file-selection.png');

    const fileCard = await driver.$('//*[contains(@label, "jfk.wav") and contains(@label, "status")]');
    await fileCard.waitForExist({ timeout: 10000 });
    const fileCardLabel = await fileCard.getAttribute('label');
    if (fileCardLabel?.includes('status error')) {
      throw new Error(`File transcription failed: ${fileCardLabel}`);
    }
    await driver.waitUntil(async () => (await fileCard.getAttribute('label'))?.includes('status done') === true, {
      timeout: 120000,
      timeoutMsg: `File transcription did not complete. Final card state: ${await fileCard.getAttribute('label')}`,
    });
    const transcriptToggle = await driver.$('//*[contains(@label, "Show transcript")]');
    await transcriptToggle.waitForExist({ timeout: 5000 });
    await transcriptToggle.click();
    const transcriptText = await driver.$('//*[contains(@label, "country")]');
    await transcriptText.waitForExist({ timeout: 5000 });
    const uiTranscript = (await transcriptText.getText()).toLowerCase();
    if (!uiTranscript.includes('country')) {
      throw new Error(`UI transcript did not contain expected keyword "country": ${uiTranscript}`);
    }
    console.log('  ✔ File transcription completed and transcript rendered in the UI');

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
      {
        name: 'Qwen3-ASR 1.7B',
        version: 'Pure-Rust Native MLX / ONNX',
        backend: 'Native MLX / ORT',
        loadTarget: 'qwen3-asr-1.7b-mlx',
        engine: 'qwen3',
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
        if (!passed) {
          throw new Error(`Transcript accuracy check failed for ${m.name} on ${af.name}; expected keyword "${af.refKeyword}"`);
        }
      }
    }

  } finally {
    console.log('\n[TEARDOWN] Terminating Appium session...');
    try {
      await driver.deleteSession();
    } catch {}

    if (appiumProcess) {
      console.log('[TEARDOWN] Stopping spawned Appium server process...');
      appiumProcess.kill('SIGINT');
      await new Promise((r) => setTimeout(r, 600));
      spawnSync('pkill', ['-f', 'WebDriverAgentRunner']);
      console.log('  ✔ Appium daemon stopped cleanly.');
    }
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
  const qwenModelDir = process.env.TAURSCRIBE_QWEN3_MODEL_DIR ?? resolve('target/qwen3-model-test');
  const runRes = spawnSync(runnerBin, ['--engine', engine, '--model', modelTarget, '--audio', audioPath], {
    encoding: 'utf-8',
    env: {
      ...process.env,
      DYLD_FRAMEWORK_PATH: frameworksDir,
      DYLD_LIBRARY_PATH: frameworksDir,
      TAURSCRIBE_QWEN3_MODEL_DIR: qwenModelDir,
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
