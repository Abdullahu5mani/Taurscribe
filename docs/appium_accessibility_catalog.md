# Taurscribe Appium & Accessibility Identifier Catalog

This document provides a comprehensive reference of all Accessibility IDs (`accessibilityId`), WAI-ARIA roles, and interaction semantics across the Taurscribe desktop application. Every interactive button, input, toggle, select, slider, modal, and status container is accessible and automatable via Appium across macOS, Windows, and WebView environments.

---

## 1. Automation Architecture & Locator Strategies

Taurscribe's cross-platform accessibility architecture uses standardized attributes so automation scripts can locate elements using the standard `accessibility id` locator strategy (`~<id>` or `AppiumBy.ACCESSIBILITY_ID`):

| Platform / Driver | Native Accessibility Protocol | Locator Strategy | Target Attribute |
| :--- | :--- | :--- | :--- |
| **macOS** (`appium-mac2-driver`) | Apple `AXUIElement` / WebKit AX | `AppiumBy.ACCESSIBILITY_ID` (`~id`) | `id` / `data-testid` (`AXIdentifier`), `aria-label` (`AXDescription`) |
| **Windows** (`appium-windows-driver`) | Microsoft UI Automation (UIA) | `AppiumBy.ACCESSIBILITY_ID` (`~id`) | `AutomationId` (`id` / `data-testid`), `Name` (`aria-label`) |
| **WebView / WebdriverIO** | W3C WebDriver / Chrome DevTools Protocol | `By.css('[data-testid="id"]')` or `By.id('id')` | `data-testid`, `id`, `role`, `aria-*` |

---

## 2. Complete Accessibility Identifier Catalog

### 2.1. TitleBar & Window Controls (`src/components/TitleBar.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `titlebar-header` | `<header>` | `banner` | Main title bar container |
| `titlebar-logo-btn` | `<button>` | `button` | "Cycle logo animation" — interactive brand logo |
| `titlebar-controls-mac` | `<div>` | `group` | "Window management" (macOS controls group) |
| `titlebar-close-btn` | `<button>` | `button` | "Close" — close window or minimize to tray |
| `titlebar-minimize-btn` | `<button>` | `button` | "Minimize" — minimize window to dock/taskbar |
| `titlebar-maximize-btn` | `<button>` | `button` | "Maximize" — toggle maximize / fullscreen |
| `titlebar-controls-win` | `<div>` | `group` | "Window management" (Windows controls group) |

---

### 2.2. Main Window & Dictation Controls (`src/App.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `mode-toggle-group` | `<div>` | `radiogroup` | "Input mode" — toggles mic dictation vs file transcription |
| `mode-toggle-mic` | `<button>` | `radio` | "Microphone dictation mode" (`aria-checked`) |
| `mode-toggle-files` | `<button>` | `radio` | "Audio file transcription mode" (`aria-checked`) |
| `record-button` | `<button>` | `button` | Primary recording trigger (`aria-pressed`, dynamic state label) |
| `mic-selector-dropdown` | `<select>` | `combobox` | "Microphone input device" selector |
| `engine-chip-button` | `<button>` | `button` | Engine picker dropdown trigger (`aria-expanded`) |
| `load-eject-btn` | `<button>` | `button` | Model load/unload quick-action toggle |
| `settings-open-btn` | `<button>` | `button` | "Open settings" button |
| `empty-state-download-cta` | `<button>` | `button` | CTA to download recommended models when none loaded |
| `accessibility-banner` | `<div>` | `alert` | macOS Accessibility permission warning banner |
| `enable-accessibility-btn` | `<button>` | `button` | "Open Settings" to enable Accessibility permission |
| `enable-input-monitoring-btn`| `<button>`| `button` | "Open Settings" to enable Input Monitoring permission |
| `restart-app-btn` | `<button>` | `button` | "Restart Taurscribe" button |
| `dismiss-accessibility-banner-btn` | `<button>` | `button` | "Dismiss permission banner" |
| `mic-permission-banner` | `<div>` | `alert` | Microphone permission warning banner |
| `grant-mic-permission-btn` | `<button>` | `button` | "Grant Access" to microphone |
| `open-mic-settings-btn` | `<button>` | `button` | "Open Settings" for microphone |
| `dismiss-mic-banner-btn` | `<button>` | `button` | "Dismiss microphone banner" |
| `silence-warning-banner` | `<div>` | `alert` | Low audio / silence detection alert banner |
| `dismiss-silence-banner-btn` | `<button>` | `button` | "Dismiss audio warning" |

---

### 2.3. Quick Settings Popover (`src/components/QuickSettings.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `qs-settings-btn` | `<button>` | `button` | "Open full settings" header shortcut |
| `qs-grammar` | `<button>` | `switch` | "Enable FlowScribe Grammar LLM" (`aria-checked`) |
| `qs-denoise` | `<button>` | `switch` | "Enable RNNoise audio suppression" (`aria-checked`) |
| `qs-overlay` | `<button>` | `switch` | "Show floating recording overlay" (`aria-checked`) |
| `qs-mute-bg` | `<button>` | `switch` | "Mute background audio while recording" (`aria-checked`) |
| `qs-style-pills` | `<div>` | `radiogroup` | "Transcription tone style" group |
| `qs-style-pill-verbatim` | `<button>` | `radio` | "Verbatim transcription style" (`aria-checked`) |
| `qs-style-pill-natural` | `<button>` | `radio` | "Natural transcription style" (`aria-checked`) |
| `qs-style-pill-casual` | `<button>` | `radio` | "Casual transcription style" (`aria-checked`) |
| `qs-style-pill-concise` | `<button>` | `radio` | "Concise transcription style" (`aria-checked`) |
| `qs-style-pill-formal` | `<button>` | `radio` | "Formal transcription style" (`aria-checked`) |
| `qs-asr-backend-group` | `<div>` | `group` | "Speech Engine Backend" selector container |
| `qs-asr-backend-gpu` | `<button>` | `button` | "Run speech engine on GPU" (`aria-pressed`) |
| `qs-asr-backend-cpu` | `<button>` | `button` | "Run speech engine on CPU" (`aria-pressed`) |
| `qs-asr-backend-hint` | `<p>` | `status` | GPU-only / VRAM status notice |
| `qs-llm-backend-group` | `<div>` | `group` | "Grammar LLM Backend" selector container |
| `qs-llm-backend-gpu` | `<button>` | `button` | "Run grammar LLM on GPU" (`aria-pressed`) |
| `qs-llm-backend-cpu` | `<button>` | `button` | "Run grammar LLM on CPU" (`aria-pressed`) |
| `qs-sound-mute-btn` | `<button>` | `button` | "Mute sound effects" / "Unmute sound effects" (`aria-pressed`) |
| `qs-volume-slider` | `<input type="range">` | `slider` | "Sound effects volume" (`aria-valuenow="0..100"`) |
| `qs-personal-dictionary-btn` | `<button>` | `button` | "Open personal dictionary in settings" |
| `qs-personal-snippets-btn` | `<button>` | `button` | "Open text snippets in settings" |

---

### 2.4. Speech Engine & Model Picker (`src/components/EnginePicker.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `engine-picker-dialog` | `<div>` | `dialog` | "Speech Engine Selector" modal (`aria-modal="true"`) |
| `engine-picker-backdrop` | `<div>` | `presentation`| Modal backdrop overlay |
| `ep-engine-row-whisper` | `<button>` | `button` | Select Whisper ASR engine |
| `ep-engine-row-parakeet` | `<button>` | `button` | Select Parakeet ASR engine |
| `ep-engine-row-granite` | `<button>` | `button` | Select Granite ASR engine |
| `ep-back-btn` | `<button>` | `button` | "Back to engine list" drill-down button |
| `ep-models-list` | `<div>` | `radiogroup` | Models list container for active engine |
| `ep-model-row-${id}` | `<button>` | `radio` | Model row selection (`aria-checked`) |
| `ep-download-${engine}-btn` | `<button>` | `button` | Open settings to download model when empty |
| `ep-unload-btn` | `<button>` | `button` | "Unload model and free VRAM" |

---

### 2.5. Transcript History Feed (`src/components/TranscriptFeed.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `transcript-feed` | `<div>` | `region` | "Transcript History" container (`aria-live="polite"`) |
| `feed-live-status` | `<div>` | `status` | Live recording / processing status indicator |
| `feed-empty-state` | `<div>` | `status` | Empty transcript list placeholder |
| `transcript-card-${id}` | `<div>` | `article` | Individual transcript card container |
| `transcript-copy-${id}` | `<button>` | `button` | "Copy transcript to clipboard" |
| `transcript-delete-${id}` | `<button>` | `button` | "Delete transcript entry" |
| `transcript-text` | `<p>` | `paragraph` | Body text of transcript entry |

---

### 2.6. Batch File Transcription Panel (`src/components/FileTranscriptionPanel.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `file-drop-zone` | `<div>` | `region` | Audio file drop and upload region |
| `file-browse-btn` | `<button>` | `button` | "Browse audio files" primary upload button |
| `file-browse-btn-compact` | `<button>` | `button` | "Add more audio files" header button |
| `file-queue-cancel-all` | `<button>` | `button` | "Cancel all pending transcriptions" |
| `file-card-${id}` | `<div>` | `article` | Per-file transcription status card |
| `file-copy-${id}` | `<button>` | `button` | "Copy transcript to clipboard" |
| `file-rerun-${id}` | `<button>` | `button` | "Re-transcribe audio file" |
| `file-retry-${id}` | `<button>` | `button` | "Retry transcription" after failure |
| `file-run-${id}` | `<button>` | `button` | "Transcribe this file now" |
| `file-remove-${id}` | `<button>` | `button` | "Remove audio file from list" |
| `file-cancel-${id}` | `<button>` | `button` | "Cancel transcription in progress" |
| `file-toggle-transcript-${id}`| `<button>`| `button` | "Toggle transcript view" (`aria-expanded`) |
| `file-transcript-text-${id}`| `<div>` | `region` | Full transcribed text body |
| `file-error-${id}` | `<p>` | `alert` | File transcription error message |

---

### 2.7. Settings Dialog & Tabs (`src/components/SettingsModal.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `settings-modal` | `<div>` | `dialog` | "Taurscribe Settings" modal (`aria-modal="true"`) |
| `settings-modal-overlay` | `<div>` | `presentation`| Modal backdrop click-away area |
| `settings-close-btn` | `<button>` | `button` | "Close settings" |
| `settings-tablist` | `<nav>` | `tablist` | "Settings sections" navigation bar |
| `settings-tab-models` | `<button>` | `tab` | "Models" tab (`aria-selected`, `aria-controls`) |
| `settings-tab-recording` | `<button>` | `tab` | "Recording" tab (`aria-selected`, `aria-controls`) |
| `settings-tab-postprocessing`| `<button>`| `tab` | "Post-Processing" tab (`aria-selected`, `aria-controls`) |
| `settings-tab-text` | `<button>` | `tab` | "Personalisation" tab (`aria-selected`, `aria-controls`) |
| `settings-tab-app` | `<button>` | `tab` | "App" tab (`aria-selected`, `aria-controls`) |
| `settings-tab-about` | `<button>` | `tab` | "About & Data" tab (`aria-selected`, `aria-controls`) |
| `settings-tabpanel-${tab}` | `<div>` | `tabpanel` | Active tab content panel |

---

### 2.8. Models Tab (`src/components/settings/ModelsTab.tsx`, `ModelRow.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `whisper-tier-select` | `<select>` | `combobox` | "Whisper model tier" (Tiny, Base, Small, Medium, Large) |
| `whisper-language-select` | `<select>` | `combobox` | "Whisper model language" (Multilingual, English) |
| `whisper-optimization-select` | `<select>` | `combobox` | "Whisper model optimization" (Quantized, Standard) |
| `whisper-quantization-help-btn` | `<button>` | `button` | "Quantization information and hardware guidance" |
| `models-group-whisper` | `<div>` | `group` | Whisper models category container |
| `models-group-parakeet` | `<div>` | `group` | Parakeet models category container |
| `models-group-granite` | `<div>` | `group` | Granite models category container |
| `models-group-postprocessing`| `<div>` | `group` | Post-processing LLM models category container |
| `model-row-${id}` | `<div>` | `region` | Model card container for model `${id}` |
| `model-download-btn-${id}` | `<button>` | `button` | "Download ${model.name}" |
| `model-cancel-download-btn-${id}`| `<button>`| `button` | "Cancel download of ${model.name}" |
| `model-delete-btn-${id}` | `<button>` | `button` | "Delete ${model.name}" |
| `model-confirm-delete-yes-${id}`| `<button>`| `button` | "Confirm deletion of ${model.name}" |
| `model-confirm-delete-no-${id}` | `<button>`| `button` | "Cancel deletion of ${model.name}" |
| `model-status-badge-${id}` | `<span>` | `status` | Model status indicator (Downloaded, Available, Active) |
| `model-delete-error-${id}` | `<p>` | `alert` | Error message on deletion failure |

---

### 2.9. Recording & Hotkey Tab (`src/components/settings/RecordingTab.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `hotkey-mode-group` | `<div>` | `radiogroup` | "Recording mode" selector |
| `hotkey-mode-hold-btn` | `<button>` | `radio` | "Hold to record mode" (`aria-checked`) |
| `hotkey-mode-toggle-btn` | `<button>` | `radio` | "Toggle to record mode" (`aria-checked`) |
| `hotkey-change-btn` | `<button>` | `button` | "Change hotkey combination" |
| `hotkey-save-btn` | `<button>` | `button` | "Save hotkey combination" |
| `hotkey-cancel-btn` | `<button>` | `button` | "Cancel hotkey change" |
| `recording-device-select` | `<select>` | `combobox` | "Microphone input device" selector |
| `recording-audio-saved` | `<span>` | `status` | "Saved ✓" confirmation |
| `recording-audio-error` | `<span>` | `alert` | Audio device error notice |
| `recording-overlay-toggle` | `<button>` | `switch` | "Enable floating recording overlay" (`aria-checked`) |
| `recording-denoise-toggle` | `<button>` | `switch` | "Enable RNNoise background suppression" (`aria-checked`) |
| `recording-mute-bg-toggle` | `<button>` | `switch` | "Mute background audio during recording" (`aria-checked`) |

---

### 2.10. Post-Processing Tab (`src/components/settings/PostProcessingTab.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `grammar-llm-toggle` | `<button>` | `switch` | "Enable FlowScribe Grammar Correction LLM" (`aria-checked`)|
| `pp-llm-backend-group` | `<div>` | `group` | "Grammar LLM hardware backend" selector |
| `pp-llm-backend-gpu` | `<button>` | `button` | "Run Grammar LLM on GPU" (`aria-pressed`) |
| `pp-llm-backend-cpu` | `<button>` | `button` | "Run Grammar LLM on CPU" (`aria-pressed`) |
| `pp-style-grid` | `<div>` | `radiogroup` | "Transcription style" grid container |
| `pp-style-btn-verbatim` | `<button>` | `radio` | "Verbatim: Exact word-for-word dictation" (`aria-checked`) |
| `pp-style-btn-natural` | `<button>` | `radio` | "Natural: Fixed punctuation and flow" (`aria-checked`) |
| `pp-style-btn-casual` | `<button>` | `radio` | "Casual: Conversational tone" (`aria-checked`) |
| `pp-style-btn-concise` | `<button>` | `radio` | "Concise: Tight, direct phrasing" (`aria-checked`) |
| `pp-style-btn-formal` | `<button>` | `radio` | "Formal: Professional and structured" (`aria-checked`) |

---

### 2.11. Personalisation & Text Tab (`src/components/settings/TextTab.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `dict-input-sounds-like` | `<input>` | `textbox` | "Spoken or misrecognized word" |
| `dict-input-correct` | `<input>` | `textbox` | "Correct replacement word" |
| `dict-add-btn` | `<button>` | `button` | "Add dictionary rule" |
| `dict-entry-${id}` | `<div>` | `region` | Dictionary rule container |
| `dict-entry-sounds-like-${id}`| `<span>` | `generic` | Spoken phrase preview |
| `dict-entry-correct-${id}` | `<span>` | `generic` | Correct phrase preview |
| `dict-delete-btn-${id}` | `<button>` | `button` | "Delete dictionary rule" |
| `snippet-input-trigger` | `<input>` | `textbox` | "Snippet trigger keyword" |
| `snippet-input-expansion` | `<textarea>`| `textbox` | "Full expansion text" |
| `snippet-add-btn` | `<button>` | `button` | "Add text snippet" |
| `snippet-entry-${id}` | `<div>` | `region` | Snippet entry container |
| `snippet-entry-trigger-${id}` | `<span>` | `generic` | Snippet trigger preview |
| `snippet-entry-expansion-${id}`| `<span>` | `generic` | Snippet expansion preview |
| `snippet-delete-btn-${id}` | `<button>` | `button` | "Delete text snippet" |

---

### 2.12. App Preferences Tab (`src/components/settings/AppTab.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `close-behavior-options` | `<div>` | `radiogroup` | "Close button behavior" options container |
| `close-behavior-tray` | `<input type="radio">` | `radio` | "Minimise to tray" (`checked`) |
| `close-behavior-quit` | `<input type="radio">` | `radio` | "Quit application" (`checked`) |
| `app-sound-mute-btn` | `<button>` | `button` | "Mute sound effects" / "Unmute sound effects" (`aria-pressed`)|
| `app-sound-volume-slider` | `<input type="range">` | `slider` | "Sound effects volume" (`aria-valuenow="0..100"`) |

---

### 2.13. About & Diagnostics Tab (`src/components/settings/AboutTab.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `open-folder-models-btn` | `<button>` | `button` | "Open models folder in file manager" |
| `open-folder-recordings-btn` | `<button>` | `button` | "Open audio recordings folder in file manager" |
| `open-folder-settings-btn` | `<button>` | `button` | "Open settings configuration folder in file manager" |
| `factory-reset-btn` | `<button>` | `button` | "Reset all settings to default" trigger |
| `factory-reset-confirm-btn` | `<button>` | `button` | "Confirm factory reset" |
| `factory-reset-cancel-btn` | `<button>` | `button` | "Cancel factory reset" |

---

### 2.14. First-Run Setup Wizard (`src/components/SetupWizard.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `wizard-welcome-begin-btn` | `<button>` | `button` | "Begin Setup" on Welcome step |
| `wizard-hardware-back-btn` | `<button>` | `button` | Back button on Hardware step |
| `wizard-hardware-next-btn` | `<button>` | `button` | Continue button on Hardware step |
| `wizard-engine-prev-btn` | `<button>` | `button` | "Previous engine slide" |
| `wizard-engine-next-btn` | `<button>` | `button` | "Next engine slide" |
| `wizard-engine-carousel-dots` | `<div>` | `tablist` | Carousel dots container |
| `wizard-engine-dot-${id}` | `<button>` | `tab` | Engine slide tab dot (`aria-selected`) |
| `wizard-engines-back-btn` | `<button>` | `button` | Back button on Engines overview step |
| `wizard-engines-next-btn` | `<button>` | `button` | Continue button on Engines overview step |
| `wizard-flowscribe-back-btn` | `<button>` | `button` | Back button on FlowScribe step |
| `wizard-flowscribe-next-btn` | `<button>` | `button` | Continue button on FlowScribe step |
| `wizard-hotkey-back-btn` | `<button>` | `button` | Back button on Hotkey step |
| `wizard-hotkey-next-btn` | `<button>` | `button` | Continue button on Hotkey step |
| `wizard-toggle-denoise` | `<button>` | `switch` | "Toggle background noise suppression" (`aria-checked`) |
| `wizard-toggle-overlay` | `<button>` | `switch` | "Toggle recording overlay pill" (`aria-checked`) |
| `wizard-toggle-mute-bg` | `<button>` | `switch` | "Toggle mute background audio" (`aria-checked`) |
| `wizard-recording-back-btn` | `<button>` | `button` | Back button on Recording Settings step |
| `wizard-recording-next-btn` | `<button>` | `button` | Continue button on Recording Settings step |
| `wizard-perm-mic-ok` | `<span>` | `status` | "Granted" badge for microphone |
| `wizard-perm-mic-restricted` | `<span>` | `status` | "Restricted by policy" badge for microphone |
| `wizard-perm-mic-settings-btn`| `<button>` | `button` | "Open Microphone Settings" |
| `wizard-perm-mic-grant-btn` | `<button>` | `button` | "Grant Microphone Access" |
| `wizard-perm-acc-ok` | `<span>` | `status` | "Granted" badge for Accessibility |
| `wizard-perm-accessibility-grant-btn` | `<button>` | `button` | "Grant Accessibility Access" |
| `wizard-perm-input-ok` | `<span>` | `status` | "Granted" badge for Input Monitoring |
| `wizard-perm-input-grant-btn` | `<button>` | `button` | "Grant Input Monitoring Access" |
| `wizard-perm-restart-notice` | `<div>` | `alert` | "Restart required" permissions changed notice |
| `wizard-perm-restart-btn` | `<button>` | `button` | "Restart Application Now" |
| `wizard-perm-back-btn` | `<button>` | `button` | Back button on Permissions step |
| `wizard-perm-next-btn` | `<button>` | `button` | Continue / Skip button on Permissions step |
| `wizard-ready-bundle-indicator`| `<div>` | `status` | Multi-file model bundle download status |
| `wizard-ready-cancel-download-btn`| `<button>`| `button` | "Cancel model download" |
| `wizard-ready-download-model-btn` | `<button>`| `button` | Download recommended model CTA |
| `wizard-ready-launch-btn` | `<button>` | `button` | "Launch Taurscribe Application" |
| `wizard-ready-skip-btn` | `<button>` | `button` | Skip inline download and open settings |

---

### 2.15. Floating Desktop Overlay (`src/OverlayApp.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `overlay-pill` | `<div>` | `status` | "Recording Overlay: Listening" (`aria-live="polite"`) |
| `overlay-time-label` | `<span>` | `generic` | Dynamic status: elapsed time or latency |

---

### 2.16. Session Notices (`src/components/SessionNoticeCard.tsx`)

| Accessibility ID / `data-testid` | Element Tag | Role / Type | Accessible Label / Purpose |
| :--- | :--- | :--- | :--- |
| `session-notice-card` | `<div>` | `alert` | Active session warning / alert container |
| `session-notice-action-${id}` | `<button>` | `button` | Action button within notice |

---

## 3. Automation Examples

### 3.1. Python (`Appium-Python-Client`)

```python
from appium import webdriver
from appium.options.mac import Mac2Options
from appium.webdriver.common.appiumby import AppiumBy
from selenium.webdriver.support.ui import WebDriverWait
from selenium.webdriver.support import expected_conditions as EC

def test_taurscribe_automation():
    options = Mac2Options()
    options.bundle_id = "com.taurscribe.app"
    options.automation_name = "mac2"
    
    driver = webdriver.Remote("http://127.0.0.1:4723", options=options)
    wait = WebDriverWait(driver, 10)

    try:
        # 1. Verify Mode Toggle
        mic_mode = wait.until(
            EC.presence_of_element_located((AppiumBy.ACCESSIBILITY_ID, "mode-toggle-mic"))
        )
        assert mic_mode.get_attribute("aria-checked") == "true" or mic_mode.is_selected()

        # 2. Open Engine Picker
        engine_chip = wait.until(
            EC.element_to_be_clickable((AppiumBy.ACCESSIBILITY_ID, "engine-chip-button"))
        )
        engine_chip.click()

        # 3. Verify Engine Picker Dialog opened
        dialog = wait.until(
            EC.visibility_of_element_located((AppiumBy.ACCESSIBILITY_ID, "engine-picker-dialog"))
        )
        assert dialog is not None

        # 4. Open Whisper models
        whisper_row = driver.find_element(AppiumBy.ACCESSIBILITY_ID, "ep-engine-row-whisper")
        whisper_row.click()

        # 5. Open Settings Dialog
        settings_btn = driver.find_element(AppiumBy.ACCESSIBILITY_ID, "settings-open-btn")
        settings_btn.click()

        # 6. Switch to Recording Tab
        recording_tab = wait.until(
            EC.element_to_be_clickable((AppiumBy.ACCESSIBILITY_ID, "settings-tab-recording"))
        )
        recording_tab.click()

        # 7. Toggle Background Audio Muting Switch
        mute_toggle = driver.find_element(AppiumBy.ACCESSIBILITY_ID, "recording-mute-bg-toggle")
        is_checked = mute_toggle.get_attribute("aria-checked")
        mute_toggle.click()
        assert mute_toggle.get_attribute("aria-checked") != is_checked

        # 8. Close Settings
        close_btn = driver.find_element(AppiumBy.ACCESSIBILITY_ID, "settings-close-btn")
        close_btn.click()

    finally:
        driver.quit()
```

### 3.2. WebdriverIO / Node.js

```typescript
import { remote } from 'webdriverio';

async function run() {
    const driver = await remote({
        path: '/',
        port: 4723,
        capabilities: {
            platformName: 'Mac',
            'appium:automationName': 'mac2',
            'appium:bundleId': 'com.taurscribe.app',
        }
    });

    // Locate by standard accessibilityId prefix (~)
    const recordBtn = await driver.$('~record-button');
    await recordBtn.waitForExist({ timeout: 5000 });
    
    // Check initial recording state
    const isPressed = await recordBtn.getAttribute('aria-pressed');
    console.log(`Current recording state: ${isPressed}`);

    // Trigger recording
    await recordBtn.click();

    // Verify overlay appears
    const overlay = await driver.$('~overlay-pill');
    await overlay.waitForDisplayed({ timeout: 2000 });

    await driver.deleteSession();
}
```

---

## 4. Maintenance & Continuous Enforcement

The test suite in `scripts/tests/test_appium_accessibility.py` is integrated into Taurscribe's test suite and CI pipeline. Any newly introduced `<button>`, `<input>`, `<select>`, `<textarea>`, or ARIA container will fail automated checks unless both `id` and `data-testid` are present and proper accessible names/roles are declared.
