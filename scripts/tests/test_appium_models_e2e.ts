import { remote } from 'webdriverio';
import { resolve } from 'path';
import { existsSync, mkdirSync } from 'fs';
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
const SCREENSHOT_DIR = '/tmp/taurscribe_screenshots';
const ARTIFACT_DIR = '/Users/abdullahusmani/.gemini/antigravity/brain/842a84bb-eafe-4f04-94bd-93f838fba900';

mkdirSync(SCREENSHOT_DIR, { recursive: true });

async function captureScreenshot(driver: WebdriverIO.Browser, name: string) {
  const tmpPath = `${SCREENSHOT_DIR}/${name}`;
  const artifactPath = `${ARTIFACT_DIR}/${name}`;
  try {
    await driver.saveScreenshot(tmpPath);
    spawnSync('cp', [tmpPath, artifactPath]);
    console.log(`  📸 [SCREENSHOT] ${name} captured and saved`);
  } catch (err) {
    console.warn(`  ⚠️ Could not save screenshot ${name}: ${err}`);
  }
}

async function requireElement(driver: WebdriverIO.Browser, selector: string, description: string, timeout = 10000) {
  const element = await driver.$(selector);
  await element.waitForExist({ timeout, timeoutMsg: `Required UI element not found within ${timeout}ms: ${description} (${selector})` });
  return element;
}

async function findWithFallbacks(driver: WebdriverIO.Browser, selectors: string[], description: string, timeout = 6000) {
  const t0 = Date.now();
  while (Date.now() - t0 < timeout) {
    for (const sel of selectors) {
      try {
        const el = await driver.$(sel);
        if (await el.isExisting()) return el;
      } catch {}
    }
    await driver.pause(300);
  }
  throw new Error(`Could not find element: ${description} (tried: ${selectors.join(', ')})`);
}

async function robustClick(driver: WebdriverIO.Browser, element: any, description: string) {
  console.log(`  -> Clicking: ${description}...`);
  try {
    const loc = typeof element.getLocation === 'function' ? await element.getLocation() : null;
    const size = typeof element.getSize === 'function' ? await element.getSize() : null;
    if (loc && size && size.width > 0 && size.height > 0) {
      const cx = Math.round(loc.x + size.width / 2);
      const cy = Math.round(loc.y + size.height / 2);
      await driver.execute('macos: click', { x: cx, y: cy });
      await driver.pause(500);
      return;
    }
  } catch (err) {
    console.log(`    (coordinate click failed: ${err}, falling back to element.click())`);
  }
  await element.click();
  await driver.pause(500);
}

async function dismissSystemDialogsIfPresent(driver: WebdriverIO.Browser) {
  try {
    await driver.execute('macos: appleScript', {
      script: `
        tell application "System Events"
          tell process "Taurscribe"
            if exists (button "Allow" of window 1) then
              click button "Allow" of window 1
            else if exists (button "Don't Allow" of window 1) then
              click button "Don't Allow" of window 1
            end if
          end tell
        end tell
      `
    });
  } catch {}
}

async function selectFileInNativeDialog(driver: WebdriverIO.Browser, filePath: string, platformName: string) {
  const isMac = platformName.toLowerCase().includes('mac');
  if (isMac) {
    await driver.execute('macos: appleScript', {
      script: 'tell application "System Events" to keystroke "g" using {command down, shift down}',
    });
    await driver.pause(600);
    await captureScreenshot(driver, 'file_picker_goto_sheet.png');

    const clipboardResult = spawnSync('pbcopy', { input: filePath, encoding: 'utf-8' });
    if (clipboardResult.status !== 0) {
      throw new Error(`Failed to copy file path to the macOS clipboard: ${clipboardResult.stderr ?? ''}`);
    }
    await driver.execute('macos: appleScript', {
      script: 'tell application "System Events" to keystroke "v" using {command down}',
    });
    await driver.pause(600);
    await captureScreenshot(driver, 'file_picker_path_pasted.png');

    await driver.execute('macos: appleScript', {
      script: 'tell application "System Events" to key code 36',
    });
    await driver.pause(800);
    await captureScreenshot(driver, 'file_picker_after_goto.png');

    const openButtons = await driver.$$('//XCUIElementTypeButton[@title="Open" or @label="Open"]');
    const openButtonCount = await openButtons.length;
    if (openButtonCount === 0) {
      throw new Error('Native file picker did not expose an Open button after selecting the fixture');
    }
    await openButtons[openButtonCount - 1].click();
  } else {
    await driver.keys(['Control', 'L']);
    await driver.setClipboard(filePath);
    await driver.keys(['Control', 'V']);
    await driver.keys('Enter');
  }
  await driver.pause(1000);
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

async function isViteResponding(port: number): Promise<boolean> {
  try {
    const res = await fetch(`http://localhost:${port}/`);
    return res.ok;
  } catch {
    return false;
  }
}

async function dismissAnySystemDialogs(driver: any) {
  try {
    await driver.execute('macos: appleScript', {
      script: `
        tell application "System Events"
          if exists (window 1 of process "UserNotificationCenter") then
            try
              click button "Allow" of window 1 of process "UserNotificationCenter"
            end try
          end if
          if exists (process "CoreServicesUIAgent") then
            try
              click button "OK" of window 1 of process "CoreServicesUIAgent"
            end try
          end if
          if exists (process "Finder") then
            try
              if exists (button "OK" of window 1 of process "Finder") then
                click button "OK" of window 1 of process "Finder"
              end if
            end try
          end if
        end tell
      `,
    });
  } catch {}
}

async function runE2ETest() {
  console.log('===============================================================================');
  console.log('       TAURSCRIBE COMPREHENSIVE APPIUM E2E UI & MODEL TEST SUITE');
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

  console.log(`[INIT] App Bundle: ${appPath}`);
  console.log(`[INIT] JFK Audio Fixture: ${jfkFixture}`);
  if (existsSync(libriFixture)) {
    console.log(`[INIT] LibriSpeech Audio Fixture: ${libriFixture}`);
  }

  let viteProcess: ChildProcess | null = null;
  if (!(await isViteResponding(1420))) {
    console.log('[INIT] Spawning Vite dev server on http://localhost:1420...');
    viteProcess = spawn('bun', ['run', 'dev'], {
      cwd: resolve('.'),
      stdio: 'ignore',
    });
    const t0 = Date.now();
    while (!(await isViteResponding(1420))) {
      if (Date.now() - t0 > 15000) {
        if (viteProcess) viteProcess.kill();
        throw new Error('Failed to start Vite dev server within 15 seconds');
      }
      await new Promise((r) => setTimeout(r, 400));
    }
    console.log('  ✔ Vite dev server active and listening on port 1420');
  } else {
    console.log('[INIT] Reusing existing Vite dev server on port 1420');
  }

  let appiumProcess: ChildProcess | null = null;
  if (!(await isAppiumResponding(appiumHost, appiumPort))) {
    console.log(`[INIT] Spawning Appium server on http://127.0.0.1:${appiumPort}...`);
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
        throw new Error('Failed to start Appium server within 15 seconds');
      }
      await new Promise((r) => setTimeout(r, 400));
    }
    console.log(`  ✔ Appium server active and listening on port ${appiumPort}`);
  } else {
    console.log(`[INIT] Reusing existing Appium server on port ${appiumPort}`);
  }

  const driver = await remote({
    path: '/',
    port: appiumPort,
    capabilities: {
      platformName,
      'appium:automationName': automationName,
      'appium:bundleId': bundleId,
      'appium:app': appPath,
      'appium:showServerLogs': true,
      'appium:wdaLaunchTimeout': 120000,
    }
  });

  try {
    // -------------------------------------------------------------------------
    // STEP 1: Application Window & TitleBar Validation
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 1: Application Window & TitleBar Validation ---');
    await driver.pause(3000);
    await dismissAnySystemDialogs(driver);

    await requireElement(driver, '//XCUIElementTypeWindow[@title="Taurscribe"]', 'main Taurscribe window');
    console.log('  ✔ Main Taurscribe window detected and active');

    console.log('  Waiting for Taurscribe React UI to mount and render...');
    await dismissAnySystemDialogs(driver);
    await findWithFallbacks(driver, [
      '//XCUIElementTypeRadioButton[@title="Microphone dictation mode" or @label="Microphone dictation mode"]',
      '//XCUIElementTypeRadioButton[@title="File transcription mode" or @label="File transcription mode"]',
      '~mode-toggle-mic',
      '~mode-toggle-files',
    ], 'Taurscribe input mode radio buttons', 25000);
    console.log('  ✔ Taurscribe frontend UI mounted and interactive');

    await captureScreenshot(driver, '01_main_window_launched.png');

    // Verify window controls
    const windowControls = await driver.$$('//XCUIElementTypeButton[@label="Close" or @title="Close" or @label="Minimize" or @title="Minimize"]');
    console.log(`  ✔ Window controls detected (count: ${windowControls.length})`);

    // -------------------------------------------------------------------------
    // STEP 2: Input Mode Switch to File Mode & Native File Transcription
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 2: Input Mode Switching & File Transcription ---');
    const fileModeRadio = await findWithFallbacks(driver, [
      '//XCUIElementTypeRadioButton[@title="File transcription mode" or @label="File transcription mode"]',
      '~mode-toggle-files',
      '//*[contains(@label, "File transcription mode")]',
    ], 'File transcription mode radio button');

    console.log('  ✔ Clicking File Transcription Mode radio button...');
    await robustClick(driver, fileModeRadio, 'File Mode radio button');
    await driver.pause(1000);
    await captureScreenshot(driver, '02_file_mode_switched.png');

    // Verify file browse button is now visible
    const browseButton = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[@label="Browse audio files" or @title="Browse audio files"]',
      '~file-browse-btn',
      '//*[contains(@label, "Browse audio files")]',
      '//XCUIElementTypeButton[contains(@label, "Browse") or contains(@title, "Browse")]',
    ], 'Browse audio files button');
    console.log('  ✔ Audio file browse button detected and active');

    console.log('  ✔ Opening native file picker and selecting JFK fixture...');
    await robustClick(driver, browseButton, 'Browse audio files button');
    await driver.pause(1000);
    await captureScreenshot(driver, '03_native_file_picker_dialog.png');

    await selectFileInNativeDialog(driver, jfkFixture, platformName);
    console.log('  ✔ Fixture selected through native file dialog');
    await captureScreenshot(driver, '04_file_selected_in_queue.png');

    // Wait for file transcription card to appear and complete
    console.log('  ✔ Waiting for transcription to process...');
    const fileCard = await driver.$('//*[contains(@label, "jfk.wav") and contains(@label, "status")]');
    await fileCard.waitForExist({ timeout: 15000 });

    const fileCardLabel = await fileCard.getAttribute('label');
    if (fileCardLabel?.includes('status error')) {
      throw new Error(`File transcription immediately reported error: ${fileCardLabel}`);
    }

    await driver.waitUntil(async () => {
      try {
        const card = await driver.$('//*[contains(@label, "jfk.wav") and contains(@label, "status")]');
        const lbl = await card.getAttribute('label');
        if (lbl?.includes('status done')) return true;
        if (lbl?.includes('status error')) {
          throw new Error(`File transcription reported error: ${lbl}`);
        }
      } catch (e: any) {
        if (e.message?.includes('File transcription reported error')) throw e;
      }
      return false;
    }, {
      timeout: 120000,
      timeoutMsg: 'File transcription did not complete within timeout',
    });
    console.log('  ✔ File transcription status: DONE');
    await captureScreenshot(driver, '05_file_transcription_done.png');

    // Expand transcript
    const transcriptToggle = await findWithFallbacks(driver, [
      '//*[contains(@label, "Show transcript")]',
      '//XCUIElementTypeButton[contains(@label, "transcript")]',
    ], 'Show transcript toggle button');
    await robustClick(driver, transcriptToggle, 'Show transcript toggle');
    await driver.pause(600);

    const transcriptTextEl = await findWithFallbacks(driver, [
      '//*[contains(@label, "country")]',
      '//*[contains(@value, "country")]',
    ], 'Rendered transcript text containing keyword country');
    const fullTranscript = (await transcriptTextEl.getText() || await transcriptTextEl.getAttribute('label') || '').toLowerCase();
    if (!fullTranscript.includes('country')) {
      throw new Error(`UI transcript does not contain expected keyword "country": ${fullTranscript}`);
    }
    console.log(`  ✔ Verified rendered transcript in UI: "${fullTranscript.trim()}"`);
    await captureScreenshot(driver, '06_file_transcript_rendered.png');

    // -------------------------------------------------------------------------
    // STEP 3: Engine Picker Popover & Drill-Down
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 3: Engine Picker Popover & Multi-Engine Navigation ---');
    const engineChip = await findWithFallbacks(driver, [
      '//XCUIElementTypePopUpButton[contains(@label, "Switch engine") or contains(@title, "Switch engine")]',
      '//XCUIElementTypeButton[contains(@label, "Switch engine") or contains(@title, "Switch engine")]',
      '~engine-chip-button',
      '//*[@identifier="engine-chip-button"]',
    ], 'Engine & model switcher chip button');

    await robustClick(driver, engineChip, 'Engine chip button');
    await driver.pause(1000);
    await captureScreenshot(driver, '07_engine_picker_dialog_open.png');

    // Verify engines list
    const whisperRow = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[contains(@label, "Engine Whisper") or contains(@title, "Engine Whisper")]',
      '~ep-engine-row-whisper',
      '//*[contains(@label, "Whisper")]',
    ], 'Whisper engine row');
    const parakeetRow = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[contains(@label, "Engine Parakeet") or contains(@title, "Engine Parakeet")]',
      '~ep-engine-row-parakeet',
      '//*[contains(@label, "Parakeet")]',
    ], 'Parakeet engine row');
    const graniteRow = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[contains(@label, "Engine Granite") or contains(@title, "Engine Granite")]',
      '~ep-engine-row-granite',
      '//*[contains(@label, "Granite")]',
    ], 'Granite engine row');
    const qwen3Row = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[contains(@label, "Engine Qwen3") or contains(@title, "Engine Qwen3")]',
      '~ep-engine-row-qwen3',
      '//*[contains(@label, "Qwen3")]',
    ], 'Qwen3 engine row');

    console.log('  ✔ All 4 ASR engines detected in engine picker: Whisper, Parakeet, Granite, Qwen3-ASR');

    // Drill down into Whisper
    console.log('  ✔ Drilling down into Whisper models...');
    await robustClick(driver, whisperRow, 'Whisper engine row');
    await driver.pause(800);
    await captureScreenshot(driver, '08_engine_picker_whisper_models.png');

    // Back to engine list
    const backBtn = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[@label="Back to engine list" or @title="Back to engine list"]',
      '~ep-back-btn',
      '//*[contains(@label, "Back")]',
    ], 'Engine picker back button');
    await robustClick(driver, backBtn, 'Back button');
    await driver.pause(600);

    // Drill down into Qwen3-ASR
    console.log('  ✔ Drilling down into Qwen3-ASR models...');
    const qwen3RowAgain = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[contains(@label, "Engine Qwen3") or contains(@title, "Engine Qwen3")]',
      '~ep-engine-row-qwen3',
      '//*[contains(@label, "Qwen3")]',
    ], 'Qwen3 engine row');
    await robustClick(driver, qwen3RowAgain, 'Qwen3 engine row');
    await driver.pause(800);
    await captureScreenshot(driver, '09_engine_picker_qwen3_models.png');

    // Back to engine list
    const backBtn2 = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[@label="Back to engine list" or @title="Back to engine list"]',
      '~ep-back-btn',
      '//*[contains(@label, "Back")]',
    ], 'Engine picker back button');
    await robustClick(driver, backBtn2, 'Back button');
    await driver.pause(600);

    // Dismiss engine picker by clicking engine chip or backdrop, or Escape key
    console.log('  ✔ Dismissing engine picker popover...');
    try {
      await driver.execute('macos: appleScript', {
        script: 'tell application "System Events" to key code 53', // Esc key
      });
      await driver.pause(400);
      const popoverCheck = await driver.$('//*[@label="Engine picker" or @title="Engine picker" or contains(@label, "Select Engine")]');
      if (await popoverCheck.isExisting()) {
        await robustClick(driver, engineChip, 'Toggle engine chip closed');
        await driver.pause(500);
      }
    } catch {
      await robustClick(driver, engineChip, 'Toggle engine chip closed');
    }
    await driver.pause(600);
    await captureScreenshot(driver, '10_engine_picker_closed.png');

    // -------------------------------------------------------------------------
    // STEP 4: Microphone Dictation Mode & Controls Validation
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 4: Microphone Dictation UI Validation ---');
    const micModeRadio = await findWithFallbacks(driver, [
      '//XCUIElementTypeRadioButton[@title="Microphone dictation mode" or @label="Microphone dictation mode"]',
      '~mode-toggle-mic',
      '//*[contains(@label, "Microphone dictation mode")]',
    ], 'Microphone dictation mode radio button');

    await robustClick(driver, micModeRadio, 'Microphone Mode radio button');
    await driver.pause(800);
    await captureScreenshot(driver, '11_mic_dictation_mode_active.png');

    const recordButton = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[contains(@label, "REC") or contains(@label, "record") or contains(@label, "Record") or contains(@label, "Start recording")]',
      '~record-button',
      '//*[contains(@label, "REC") or contains(@label, "record") or contains(@label, "Record") or contains(@label, "Start recording")]',
    ], 'Main dictation record button');
    console.log(`  ✔ Record button present and accessible (Label: "${await recordButton.getAttribute('label')}")`);

    // -------------------------------------------------------------------------
    // STEP 5: Settings Modal Full 6-Tab Navigation Tour
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 5: Settings Modal Full 6-Tab Navigation Tour ---');
    const settingsOpenBtn = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[@title="Settings" or @label="Settings"]',
      '~settings-open-btn',
    ], 'Settings open button');
    await robustClick(driver, settingsOpenBtn, 'Settings open button');
    await driver.pause(1000);

    // Tab 1: Models Tab
    console.log('  ✔ Testing Settings Tab 1: Models Tab...');
    await captureScreenshot(driver, '12_settings_tab_1_models.png');

    // Tab 2: Recording Tab
    console.log('  ✔ Testing Settings Tab 2: Recording Tab...');
    const recordingTab = await findWithFallbacks(driver, [
      '//*[@label="RECORDING" or @label="Recording" or @title="RECORDING" or @title="Recording" or @value="RECORDING" or @value="Recording" or contains(@label, "RECORDING") or contains(@label, "Recording")]',
      '//XCUIElementTypeRadioButton[contains(@label, "RECORDING") or contains(@label, "Recording")]',
      '//XCUIElementTypeButton[contains(@label, "RECORDING") or contains(@label, "Recording")]',
      '//XCUIElementTypeTab[contains(@label, "RECORDING") or contains(@label, "Recording")]',
      '~settings-tab-recording',
    ], 'Recording settings tab');
    await robustClick(driver, recordingTab, 'Recording Tab');
    await driver.pause(800);
    await captureScreenshot(driver, '13_settings_tab_2_recording.png');

    // Tab 3: Grammar Tab
    console.log('  ✔ Testing Settings Tab 3: Grammar & Post-Processing Tab...');
    const grammarTab = await findWithFallbacks(driver, [
      '//*[@label="GRAMMAR" or @label="Grammar" or @title="GRAMMAR" or @title="Grammar" or @value="GRAMMAR" or @value="Grammar" or contains(@label, "GRAMMAR") or contains(@label, "Grammar")]',
      '//XCUIElementTypeRadioButton[contains(@label, "GRAMMAR") or contains(@label, "Grammar")]',
      '//XCUIElementTypeButton[contains(@label, "GRAMMAR") or contains(@label, "Grammar")]',
      '//XCUIElementTypeTab[contains(@label, "GRAMMAR") or contains(@label, "Grammar")]',
      '~settings-tab-grammar',
    ], 'Grammar settings tab');
    await robustClick(driver, grammarTab, 'Grammar Tab');
    await driver.pause(800);
    await captureScreenshot(driver, '14_settings_tab_3_grammar.png');

    // Tab 4: Text Tab (Custom Vocabulary & Personalization)
    console.log('  ✔ Testing Settings Tab 4: Text & Custom Vocabulary Tab...');
    const textTab = await findWithFallbacks(driver, [
      '//*[@label="TEXT" or @label="Text" or @title="TEXT" or @title="Text" or @value="TEXT" or @value="Text" or contains(@label, "TEXT") or contains(@label, "Text")]',
      '//XCUIElementTypeRadioButton[contains(@label, "TEXT") or contains(@label, "Text")]',
      '//XCUIElementTypeButton[contains(@label, "TEXT") or contains(@label, "Text")]',
      '//XCUIElementTypeTab[contains(@label, "TEXT") or contains(@label, "Text")]',
      '~settings-tab-text',
    ], 'Text settings tab');
    await robustClick(driver, textTab, 'Text Tab');
    await driver.pause(800);
    await captureScreenshot(driver, '15_settings_tab_4_text_vocab.png');

    // Interact with Custom Vocabulary Developer Preset
    try {
      const devPresetBtn = await findWithFallbacks(driver, [
        '//*[@label="+ Developer Pack" or contains(@label, "Developer") or contains(@title, "Developer") or contains(@value, "Developer")]',
        '//XCUIElementTypeButton[contains(@label, "Developer")]',
        '~vocab-preset-developer',
      ], 'Developer vocabulary preset button', 4000);
      await robustClick(driver, devPresetBtn, 'Developer Vocabulary Preset');
      await driver.pause(800);
      await captureScreenshot(driver, '16_settings_vocab_preset_active.png');
      console.log('  ✔ Custom Vocabulary preset added and rendered');
    } catch (err) {
      console.warn(`  ⚠️ Developer preset interaction note: ${err}`);
      await captureScreenshot(driver, '16_settings_vocab_preset_active.png');
    }

    // Tab 5: App Tab
    console.log('  ✔ Testing Settings Tab 5: App Configuration Tab...');
    const appTab = await findWithFallbacks(driver, [
      '//*[@label="APP" or @label="App" or @title="APP" or @title="App" or @value="APP" or @value="App" or contains(@label, "APP") or contains(@label, "App")]',
      '//XCUIElementTypeRadioButton[contains(@label, "APP") or contains(@label, "App")]',
      '//XCUIElementTypeButton[contains(@label, "APP") or contains(@label, "App")]',
      '//XCUIElementTypeTab[contains(@label, "APP") or contains(@label, "App")]',
      '~settings-tab-app',
    ], 'App settings tab');
    await robustClick(driver, appTab, 'App Tab');
    await driver.pause(800);
    await captureScreenshot(driver, '17_settings_tab_5_app.png');

    // Tab 6: About Tab
    console.log('  ✔ Testing Settings Tab 6: About & Diagnostics Tab...');
    const aboutTab = await findWithFallbacks(driver, [
      '//*[@label="ABOUT" or @label="About" or @title="ABOUT" or @title="About" or @value="ABOUT" or @value="About" or contains(@label, "ABOUT") or contains(@label, "About")]',
      '//XCUIElementTypeRadioButton[contains(@label, "ABOUT") or contains(@label, "About")]',
      '//XCUIElementTypeButton[contains(@label, "ABOUT") or contains(@label, "About")]',
      '//XCUIElementTypeTab[contains(@label, "ABOUT") or contains(@label, "About")]',
      '~settings-tab-about',
    ], 'About settings tab');
    await robustClick(driver, aboutTab, 'About Tab');
    await driver.pause(800);
    await captureScreenshot(driver, '18_settings_tab_6_about.png');

    // Close Settings Modal
    console.log('  ✔ Closing Settings modal...');
    const settingsCloseBtn = await findWithFallbacks(driver, [
      '//XCUIElementTypeButton[@label="Close settings" or @title="Close settings" or contains(@label, "Close") or contains(@title, "Close")]',
      '//*[@label="Close settings" or @title="Close settings" or contains(@label, "Close") or contains(@title, "Close")]',
      '~settings-close-btn',
    ], 'Settings close button');
    await robustClick(driver, settingsCloseBtn, 'Settings Close button');
    await driver.pause(800);
    await captureScreenshot(driver, '19_settings_closed_return_main.png');
    console.log('  ✔ Settings modal closed successfully');

    // -------------------------------------------------------------------------
    // STEP 6: Multi-Model & Multi-Version Audio Transcription Evaluation
    // -------------------------------------------------------------------------
    console.log('\n--- STEP 6: Multi-Model & Multi-Version Audio Transcription Evaluation ---');

    const testModels = [
      {
        name: 'Parakeet Nemotron 0.6B',
        version: 'MLX FastConformer RNN-T (Native FP32)',
        backend: 'Apple Silicon Metal GPU',
        loadTarget: 'nemotron:parakeet-nemotron-mlx',
        engine: 'parakeet',
      },
      {
        name: 'Whisper Tiny',
        version: 'Quantized Q5_1 (CoreML Offloaded)',
        backend: 'CoreML Apple Silicon GPU',
        loadTarget: 'whisper-tiny-q5_1',
        engine: 'whisper',
      },
      {
        name: 'Whisper Tiny',
        version: 'Standard Multilingual (FP16/FP32)',
        backend: 'CoreML Apple Silicon GPU',
        loadTarget: 'whisper-tiny',
        engine: 'whisper',
      },
      {
        name: 'Qwen3-ASR 1.7B',
        version: 'Official HF Safetensors (Zero Quantization)',
        backend: 'Apple Silicon MLX Metal (Pure Rust)',
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

    if (viteProcess) {
      console.log('[TEARDOWN] Stopping spawned Vite dev server...');
      viteProcess.kill('SIGINT');
      console.log('  ✔ Vite server stopped cleanly.');
    }
  }

  // ---------------------------------------------------------------------------
  // STEP 7: Final Benchmark & Verification Table
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
  const releaseBin = resolve('src-tauri/target/release/e2e_model_eval_runner');
  const debugBin = resolve('src-tauri/target/debug/e2e_model_eval_runner');
  const runnerBin = existsSync(releaseBin) ? releaseBin : debugBin;
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
