import { useState, useEffect, useRef, useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  IconVideo,
  IconRecord,
  IconMic,
  IconSearch,
  IconX,
  IconUsers,
  IconClock,
  IconPlay,
  IconStop,
  IconDownloadFile,
  IconTrash,
  IconCopy,
  IconEdit,
  IconCheck,
  IconChevronRight,
  IconChevronDown,
  IconExternalLink,
  IconRefresh,
} from "./Icons";
import type { MeetingInfo } from "./MeetingHeaderPill";
import {
  MeetingSummaryRecord,
  MeetingDetailRecord,
  PlatformCount,
} from "../types/meetings";
import { SpeakerVaultModal } from "./SpeakerVaultModal";
import { MeetingReviewModal } from "./MeetingReviewModal";
import { levelToPercent } from "../utils/audioLevel";
import "./MeetingsPanel.css";

interface MeetingsPanelProps {
  activeMeeting: MeetingInfo | null;
  isRecording: boolean;
  isDualChannelRecording: boolean;
  dualLevels: { mic: number; system: number };
  onStartDualRecording: () => void;
  onStopRecording: () => void;
  onOpenSettings?: () => void;
  /** Opens Settings → Models (speaker recognition model for the Speaker Vault). */
  onOpenModelSettings?: () => void;
}



const PLATFORM_META: Record<string, { label: string; color: string; bg: string; border: string }> = {
  "Google Meet": { label: "Google Meet", color: "#10b981", bg: "rgba(16, 185, 129, 0.14)", border: "rgba(16, 185, 129, 0.3)" },
  "Zoom": { label: "Zoom", color: "#3b82f6", bg: "rgba(59, 130, 246, 0.14)", border: "rgba(59, 130, 246, 0.3)" },
  "Microsoft Teams": { label: "Teams", color: "#8b5cf6", bg: "rgba(139, 92, 246, 0.14)", border: "rgba(139, 92, 246, 0.3)" },
  "Slack": { label: "Slack", color: "#f59e0b", bg: "rgba(245, 158, 11, 0.14)", border: "rgba(245, 158, 11, 0.3)" },
  "Webex": { label: "Webex", color: "#06b6d4", bg: "rgba(6, 182, 212, 0.14)", border: "rgba(6, 182, 212, 0.3)" },
  "Discord": { label: "Discord", color: "#6366f1", bg: "rgba(99, 102, 241, 0.14)", border: "rgba(99, 102, 241, 0.3)" },
  "Direct Audio": { label: "Direct Audio", color: "#ec4899", bg: "rgba(236, 72, 153, 0.14)", border: "rgba(236, 72, 153, 0.3)" },
};

// Meetings are saved under the detector's platform keys ("meet", "zoom", …).
const PLATFORM_KEY_NAMES: Record<string, string> = {
  meet: "Google Meet",
  zoom: "Zoom",
  teams: "Microsoft Teams",
  slack: "Slack",
  webex: "Webex",
  discord: "Discord",
};

function getPlatformMeta(plat: string) {
  return (
    PLATFORM_META[plat] || PLATFORM_META[PLATFORM_KEY_NAMES[plat.toLowerCase()] ?? ""] || {
      label: plat,
      color: "#94a3b8",
      bg: "rgba(148, 163, 184, 0.14)",
      border: "rgba(148, 163, 184, 0.3)",
    }
  );
}

export function MeetingsPanel({
  activeMeeting,
  isRecording,
  isDualChannelRecording,
  dualLevels,
  onStartDualRecording,
  onStopRecording,
  onOpenModelSettings,
}: MeetingsPanelProps) {
  // Meetings Catalog
  const [meetings, setMeetings] = useState<MeetingSummaryRecord[]>([]);
  const [selectedMeetingId, setSelectedMeetingId] = useState<number | null>(null);
  const [selectedMeeting, setSelectedMeeting] = useState<MeetingDetailRecord | null>(null);
  const [loadingList, setLoadingList] = useState(false);
  const [loadingDetail, setLoadingDetail] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedPlatform, setSelectedPlatform] = useState<string>("All");
  const searchQueryRef = useRef(searchQuery);
  const selectedPlatformRef = useRef(selectedPlatform);
  searchQueryRef.current = searchQuery;
  selectedPlatformRef.current = selectedPlatform;
  const selectedMeetingIdRef = useRef<number | null>(null);
  const listRequestRef = useRef(0);
  const detailRequestRef = useRef(0);
  const [platformCounts, setPlatformCounts] = useState<PlatformCount[]>([]);
  const [groupByPlatform, setGroupByPlatform] = useState<boolean>(false);
  const [collapsedPlatforms, setCollapsedPlatforms] = useState<Record<string, boolean>>({});

  // Audio Playback State (Full Recording Scrubber)
  const [isPlayingAudio, setIsPlayingAudio] = useState(false);
  const [audioFailed, setAudioFailed] = useState(false);
  const [audioCurrentTime, setAudioCurrentTime] = useState(0);
  const [audioDuration, setAudioDuration] = useState(0);
  const [playbackSpeed, setPlaybackSpeed] = useState<number>(1.0);
  const fullAudioRef = useRef<HTMLAudioElement | null>(null);

  // Speaker Snippet Preview Playback
  const [playingSnippetSpeakerId, setPlayingSnippetSpeakerId] = useState<string | null>(null);
  const snippetAudioRef = useRef<HTMLAudioElement | null>(null);

  // Modals & Renaming
  const [isVaultOpen, setIsVaultOpen] = useState(false);
  const [reviewMeetingId, setReviewMeetingId] = useState<number | null>(null);
  // A stopped call being transcribed + diarized (between the backend's
  // meeting-processing-started and meeting-processing-finished events).
  const [processing, setProcessing] = useState<{ title: string; platform: string; startedAt: number } | null>(null);
  const [processingError, setProcessingError] = useState<string | null>(null);
  const [processingElapsed, setProcessingElapsed] = useState(0);
  const [editingTitle, setEditingTitle] = useState("");
  const [renamingSpeakerId, setRenamingSpeakerId] = useState<string | null>(null);
  // Speaker ids like "speaker_remote_1" repeat across meetings: close any open
  // rename field when another meeting is opened.
  useEffect(() => {
    setRenamingSpeakerId(null);
  }, [selectedMeetingId]);
  const [speakerNewName, setSpeakerNewName] = useState("");
  const [copiedTranscript, setCopiedTranscript] = useState(false);

  const selectMeeting = (id: number | null) => {
    if (selectedMeetingIdRef.current !== id) {
      detailRequestRef.current += 1;
      selectedMeetingIdRef.current = id;
      setSelectedMeeting(null);
      setEditingTitle("");
      setLoadingDetail(false);
    }
    setSelectedMeetingId(id);
  };

  // Fetch list of meetings and platform counts
  const loadMeetings = async () => {
    const request = ++listRequestRef.current;
    setLoadingList(true);
    try {
      const [list, counts] = await Promise.all([
        invoke<MeetingSummaryRecord[]>("list_meetings", {
          search: searchQueryRef.current.trim() || null,
          category: null,
          platform: selectedPlatformRef.current === "All" ? null : selectedPlatformRef.current,
          limit: 100,
          offset: 0,
        }),
        invoke<PlatformCount[]>("get_meeting_platform_counts").catch(() => []),
      ]);
      if (request !== listRequestRef.current) return;
      setMeetings(list);
      setPlatformCounts(counts || []);

      // Auto-select first meeting on wider screens if none selected or if previously selected was deleted
      if (list.length > 0) {
        if (!selectedMeetingIdRef.current || !list.some((m) => m.id === selectedMeetingIdRef.current)) {
          if (window.innerWidth >= 768) {
            selectMeeting(list[0].id);
          }
        }
      } else {
        selectMeeting(null);
      }
    } catch (err) {
      console.error("Failed to load meetings list:", err);
    } finally {
      if (request === listRequestRef.current) setLoadingList(false);
    }
  };

  // Dynamic available platforms list
  // Filter values are the stored platform keys (the backend filters on them);
  // labels come from getPlatformMeta. Mixing in display names used to add a
  // second "Zoom" chip next to "zoom" that matched nothing.
  const availablePlatforms = useMemo(() => {
    const standard = ["Google Meet", "Zoom", "Microsoft Teams", "Slack", "Webex", "Discord", "Direct Audio"];
    if (platformCounts.length === 0) {
      return ["All", ...standard];
    }
    const stored = platformCounts.filter((c) => c.count > 0).map((c) => c.platform);
    return ["All", ...Array.from(new Set(stored))];
  }, [platformCounts]);

  // Grouped meetings by platform
  const groupedMeetings = useMemo(() => {
    const groups: Record<string, MeetingSummaryRecord[]> = {};
    for (const m of meetings) {
      const plat = m.platform || "Direct Audio";
      if (!groups[plat]) {
        groups[plat] = [];
      }
      groups[plat].push(m);
    }
    return groups;
  }, [meetings]);

  const toggleCollapsePlatform = (plat: string) => {
    setCollapsedPlatforms((prev) => ({
      ...prev,
      [plat]: !prev[plat],
    }));
  };

  // Fetch detail for selected meeting
  const loadMeetingDetail = async (id: number) => {
    if (selectedMeetingIdRef.current !== id) return;
    const request = ++detailRequestRef.current;
    setLoadingDetail(true);
    stopFullAudio();
    stopSnippetAudio();
    try {
      const detail = await invoke<MeetingDetailRecord>("get_meeting_detail", {
        meetingId: id,
      });
      if (request !== detailRequestRef.current || selectedMeetingIdRef.current !== id) return;
      setSelectedMeeting(detail);
      setAudioFailed(false);
      setEditingTitle(detail.title);
    } catch (err) {
      if (request === detailRequestRef.current) console.error("Failed to load meeting detail:", err);
    } finally {
      if (request === detailRequestRef.current) setLoadingDetail(false);
    }
  };

  useEffect(() => {
    loadMeetings();
  }, [searchQuery, selectedPlatform]);

  useEffect(() => {
    if (selectedMeetingId !== null) {
      loadMeetingDetail(selectedMeetingId);
    }
  }, [selectedMeetingId]);

  // Processing indicator lifecycle
  useEffect(() => {
    const unlisteners: Promise<() => void>[] = [
      listen<{ title: string; platform: string }>("meeting-processing-started", (event) => {
        setProcessingError(null);
        setProcessingElapsed(0);
        setProcessing({
          title: event.payload?.title || "Meeting",
          platform: event.payload?.platform || "",
          startedAt: Date.now(),
        });
      }),
      listen<{ meeting_id: number | null; error: string | null }>("meeting-processing-finished", (event) => {
        setProcessing(null);
        if (event.payload?.error) {
          setProcessingError(event.payload.error);
        } else if (!event.payload?.meeting_id) {
          setProcessingError("Nothing was saved: no speech was transcribed from the recording.");
        }
        loadMeetings();
      }),
    ];
    return () => {
      unlisteners.forEach((u) => u.then((fn) => fn()));
    };
  }, []);

  useEffect(() => {
    if (!processing) return;
    const timer = setInterval(() => {
      setProcessingElapsed(Math.floor((Date.now() - processing.startedAt) / 1000));
    }, 1000);
    return () => clearInterval(timer);
  }, [processing]);

  // Listen for backend meeting processing completion event
  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen<{ meeting_id: number }>("meeting-processing-complete", (event) => {
      const id = event.payload?.meeting_id;
      if (id) {
        setReviewMeetingId(id);
        loadMeetings();
      }
    }).then((u) => {
      unlisten = u;
    });

    return () => {
      if (unlisten) unlisten();
      stopFullAudio();
      stopSnippetAudio();
    };
  }, []);

  // ── Full Audio Player Controls ──────────────────────────────────
  const stopFullAudio = () => {
    if (fullAudioRef.current) {
      fullAudioRef.current.pause();
      fullAudioRef.current = null;
    }
    setIsPlayingAudio(false);
  };

  const handleToggleFullAudio = () => {
    if (!selectedMeeting?.audio_path) return;

    if (isPlayingAudio) {
      fullAudioRef.current?.pause();
      setIsPlayingAudio(false);
    } else {
      if (!fullAudioRef.current) {
        const url = convertFileSrc(selectedMeeting.audio_path);
        const audio = new Audio(url);
        // The file can vanish after the list was loaded.
        audio.onerror = () => {
          if (fullAudioRef.current !== audio) return;
          setIsPlayingAudio(false);
          fullAudioRef.current = null;
          setAudioFailed(true);
        };
        audio.playbackRate = playbackSpeed;
        audio.currentTime = audioCurrentTime;

        audio.ontimeupdate = () => {
          if (fullAudioRef.current !== audio) return;
          setAudioCurrentTime(audio.currentTime);
          setAudioDuration(audio.duration || 0);
        };

        audio.onended = () => {
          if (fullAudioRef.current !== audio) return;
          setIsPlayingAudio(false);
          setAudioCurrentTime(0);
        };

        fullAudioRef.current = audio;
      }
      const audio = fullAudioRef.current;
      audio.play().then(() => {
        if (fullAudioRef.current === audio && !audio.paused) setIsPlayingAudio(true);
      }).catch((error) => {
        console.error("Meeting audio playback failed:", error);
        if (fullAudioRef.current === audio) {
          fullAudioRef.current = null;
          setAudioFailed(true);
          setIsPlayingAudio(false);
        }
      });
    }
  };

  const handleScrubAudio = (targetSec: number) => {
    setAudioCurrentTime(targetSec);
    if (fullAudioRef.current) {
      fullAudioRef.current.currentTime = targetSec;
    }
  };

  const handleCycleSpeed = () => {
    const nextSpeed = playbackSpeed === 1 ? 1.25 : playbackSpeed === 1.25 ? 1.5 : 1;
    setPlaybackSpeed(nextSpeed);
    if (fullAudioRef.current) {
      fullAudioRef.current.playbackRate = nextSpeed;
    }
  };

  // ── Snippet Audio Player Controls (3s clip) ─────────────────────
  const stopSnippetAudio = () => {
    if (snippetAudioRef.current) {
      snippetAudioRef.current.pause();
      snippetAudioRef.current = null;
    }
    setPlayingSnippetSpeakerId(null);
  };

  const handlePlaySnippet = (speakerId: string, snippetPath?: string | null, replace = false) => {
    if (!replace && playingSnippetSpeakerId === speakerId) {
      stopSnippetAudio();
      return;
    }
    stopSnippetAudio();
    if (!snippetPath) return;

    try {
      const assetUrl = convertFileSrc(snippetPath);
      const audio = new Audio(assetUrl);
      snippetAudioRef.current = audio;
      setPlayingSnippetSpeakerId(speakerId);

      audio.onended = () => {
        if (snippetAudioRef.current !== audio) return;
        setPlayingSnippetSpeakerId(null);
        snippetAudioRef.current = null;
      };

      audio.onerror = () => {
        if (snippetAudioRef.current !== audio) return;
        setPlayingSnippetSpeakerId(null);
        snippetAudioRef.current = null;
      };

      audio.play().catch((error) => {
        console.error("Snippet playback failed:", error);
        if (snippetAudioRef.current === audio) stopSnippetAudio();
      });
    } catch (e) {
      console.error("Snippet play error:", e);
    }
  };

  const handleCycleSnippet = async (meetingId: number, speakerId: string) => {
    try {
      const [nextPath, nextIdx] = await invoke<[string, number]>("cycle_speaker_turn_snippet", {
        meetingId,
        speakerId,
      });

      if (selectedMeetingIdRef.current === meetingId) {
        setSelectedMeeting((prev) => prev?.id === meetingId ? {
          ...prev,
          turns: prev.turns.map((t) => t.speaker_id === speakerId ? {
            ...t,
            snippet_path: nextPath,
            current_snippet_idx: nextIdx,
          } : t),
        } : prev);
      }

      // If this speaker's snippet was currently playing, immediately play the new candidate sample
      if (selectedMeetingIdRef.current === meetingId && playingSnippetSpeakerId === speakerId) {
        handlePlaySnippet(speakerId, nextPath, true);
      }
    } catch (err) {
      console.error("Failed to cycle speaker snippet:", err);
    }
  };

  // ── Detail Updates ─────────────────────────────────────────────
  const handleSaveTitle = async () => {
    if (!selectedMeeting || !editingTitle.trim() || editingTitle === selectedMeeting.title) {
      return;
    }
    const meetingId = selectedMeeting.id;
    const newTitle = editingTitle.trim();
    try {
      await invoke("update_meeting", {
        meetingId,
        title: newTitle,
        category: selectedMeeting.category,
        summary: selectedMeeting.summary,
        actionItems: selectedMeeting.action_items,
      });
      if (selectedMeetingIdRef.current === meetingId) {
        setSelectedMeeting((prev) => (prev?.id === meetingId ? { ...prev, title: newTitle } : prev));
      }
      loadMeetings();
    } catch (err) {
      console.error("Failed to update meeting title:", err);
    }
  };

  const handleRenameSpeaker = async (speakerId: string) => {
    if (!selectedMeeting || !speakerNewName.trim()) {
      setRenamingSpeakerId(null);
      return;
    }

    try {
      await invoke("rename_speaker", {
        meetingId: selectedMeeting.id,
        speakerId,
        newName: speakerNewName.trim(),
        updateVault: true,
      });
      setRenamingSpeakerId(null);
      loadMeetingDetail(selectedMeeting.id);
    } catch (err) {
      console.error("Failed to rename speaker:", err);
    }
  };

  const handleDeleteMeeting = async () => {
    if (!selectedMeeting || selectedMeeting.id !== selectedMeetingIdRef.current) return;
    const meetingId = selectedMeeting.id;
    try {
      await invoke("delete_meeting", { meetingId });
      if (selectedMeetingIdRef.current === meetingId) selectMeeting(null);
      loadMeetings();
    } catch (err) {
      console.error("Failed to delete meeting:", err);
    }
  };

  const handleExport = async (format: "markdown" | "text" | "json") => {
    if (!selectedMeeting) return;
    try {
      const content = await invoke<string>("export_meeting_notes", {
        meetingId: selectedMeeting.id,
        format,
      });
      const blob = new Blob([content], {
        type: format === "json" ? "application/json" : "text/plain",
      });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${selectedMeeting.title.replace(/\s+/g, "_")}_transcript.${
        format === "markdown" ? "md" : format === "json" ? "json" : "txt"
      }`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Failed to export meeting notes:", err);
    }
  };

  const handleCopyTranscript = () => {
    if (!selectedMeeting) return;
    const text =
      selectedMeeting.turns && selectedMeeting.turns.length > 0
        ? selectedMeeting.turns
            .map(
              (t) =>
                `[${formatSeconds(Math.round(t.start_ms / 1000))}] ${t.speaker_name}:\n${t.text}`
            )
            .join("\n\n")
        : selectedMeeting.transcript_raw;
    navigator.clipboard.writeText(text);
    setCopiedTranscript(true);
    setTimeout(() => setCopiedTranscript(false), 2000);
  };

  const formatSeconds = (sec: number) => {
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s < 10 ? "0" : ""}${s}`;
  };

  const formatDurationMs = (ms: number) => {
    const totalSec = Math.round(ms / 1000);
    const m = Math.floor(totalSec / 60);
    const s = totalSec % 60;
    if (m === 0) return `${s}s`;
    return `${m}m ${s < 10 ? "0" : ""}${s}s`;
  };

  const isRecordingCall = isRecording && isDualChannelRecording;

  // Level meters percentage
  const micPercent = levelToPercent(dualLevels.mic);
  const sysPercent = levelToPercent(dualLevels.system);

  return (
    <div className="meetings-panel" data-testid="meetings-panel">
      {/* ── Active Live Call Banner (If call is detected or recording) ─ */}
      {activeMeeting && (
        <div className={`meeting-live-banner ${isRecordingCall ? "recording" : ""} ${selectedMeetingId ? "meeting-live-banner--has-selection" : ""}`}>
          <div className="meeting-live-left">
            <div className="meeting-live-pulse-dot" />
            <div className="meeting-live-meta">
              <div className="meeting-live-title-row">
                <span className="meeting-live-title">
                  {activeMeeting.title.trim() || `${activeMeeting.platform} Call`}
                </span>
                <span className="meeting-card-platform-badge">
                  {activeMeeting.platform.toUpperCase()}
                </span>
              </div>
              <span className="meeting-live-sub">
                {isRecordingCall ? "Dual-channel recording in progress..." : "Live meeting detected"}
                {" • "}
                {activeMeeting.app_name} (PID {activeMeeting.pid})
              </span>
            </div>
          </div>

          <div>
            {isRecordingCall ? (
              <button
                id="meetings-stop-recording-btn"
                data-testid="meetings-stop-recording-btn"
                type="button"
                className="meeting-live-btn meeting-live-btn--stop"
                onClick={onStopRecording}
              >
                <IconStop size={14} />
                <span>Stop & Transcribe Call</span>
              </button>
            ) : (
              <button
                id="meetings-record-call-btn"
                data-testid="meetings-record-call-btn"
                type="button"
                className="meeting-live-btn meeting-live-btn--record"
                onClick={onStartDualRecording}
              >
                <IconRecord size={14} />
                <span>Record Call (Mic + Audio)</span>
              </button>
            )}
          </div>
        </div>
      )}

      {/* ── Audio Meters (Visible when recording or call is active) ── */}
      {(isRecordingCall || activeMeeting) && (
        <div className={`meeting-meters-card ${selectedMeetingId ? "meeting-meters-card--has-selection" : ""}`}>
          <div className="meeting-meters__header">
            <h4 className="meeting-meters__title">Dual-Channel Audio Telemetry</h4>
            <span className="meeting-meters__tag">48 kHz Lossless Stereo</span>
          </div>
          <div className="meeting-meters__track">
            <div className="meeting-meter-row">
              <div className="meeting-meter-label">
                <IconMic size={13} />
                <span>Left: Your Microphone</span>
              </div>
              <div className="meeting-meter-bar-container">
                <div
                  className="meeting-meter-bar meeting-meter-bar--mic"
                  style={{ width: `${isRecording ? Math.max(4, micPercent) : 2}%` }}
                />
              </div>
              <span className="meeting-meter-val">{isRecording ? `${micPercent}%` : "0%"}</span>
            </div>

            <div className="meeting-meter-row">
              <div className="meeting-meter-label">
                <IconVideo size={13} />
                <span>Right: Meeting Callers</span>
              </div>
              <div className="meeting-meter-bar-container">
                <div
                  className="meeting-meter-bar meeting-meter-bar--sys"
                  style={{ width: `${isRecording ? Math.max(4, sysPercent) : 2}%` }}
                />
              </div>
              <span className="meeting-meter-val">{isRecording ? `${sysPercent}%` : "0%"}</span>
            </div>
          </div>
        </div>
      )}

      {/* ── Search & Filter Controls ─────────────────────────── */}
      <div className={`meeting-filter-bar ${selectedMeetingId ? "meeting-filter-bar--has-selection" : ""}`}>
        <div className="meeting-search-row">
          <div className="meeting-search-input-wrap">
            <IconSearch size={15} className="meeting-search-icon" />
            <input
              aria-label="Search meetings"
              id="meetings-search-input"
              data-testid="meetings-search-input"
              type="text"
              className="meeting-search-input"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder="Search meetings..."
            />
            {searchQuery && (
              <button
                id="meetings-search-clear-btn"
                data-testid="meetings-search-clear-btn"
                type="button"
                className="meeting-search-clear"
                onClick={() => setSearchQuery("")}
                aria-label="Clear search"
              >
                <IconX size={14} />
              </button>
            )}
          </div>

          <div className="meeting-view-mode-toggle">
            <button
              id="meetings-view-recent-btn"
              data-testid="meetings-view-recent-btn"
              type="button"
              className={`view-mode-btn ${!groupByPlatform ? "active" : ""}`}
              onClick={() => setGroupByPlatform(false)}
              title="Show meetings in chronological date order"
            >
              Recent
            </button>
            <button
              id="meetings-view-platform-btn"
              data-testid="meetings-view-platform-btn"
              type="button"
              className={`view-mode-btn ${groupByPlatform ? "active" : ""}`}
              onClick={() => setGroupByPlatform(true)}
              title="Organize meetings grouped under platform sections"
            >
              Platform
            </button>
          </div>

          <button
            id="meetings-vault-btn"
            data-testid="meetings-vault-btn"
            type="button"
            className="meeting-vault-btn"
            onClick={() => setIsVaultOpen(true)}
            title="Manage identified speakers & voiceprints"
          >
            <IconUsers size={13} />
            <span className="vault-btn-text">Speaker Vault</span>
          </button>
        </div>

        {/* Platform Filter Pills */}
        <div className="meeting-filter-pills-row">
          <span className="filter-pill-label">Platform:</span>
          <div className="meeting-pills-scroll">
            {availablePlatforms.map((plat) => {
              const isAll = plat === "All";
              const meta = getPlatformMeta(plat);
              const count = isAll
                ? meetings.length
                : platformCounts.find((c) => c.platform === plat)?.count;

              return (
                <button
                  id={`meetings-platform-tab-${plat}`}
                  data-testid={`meetings-platform-tab-${plat}`}
                  key={plat}
                  type="button"
                  className={`platform-tab-btn ${selectedPlatform === plat ? "active" : ""}`}
                  style={
                    selectedPlatform === plat && !isAll
                      ? { borderColor: meta.color, color: meta.color, background: meta.bg }
                      : undefined
                  }
                  onClick={() => setSelectedPlatform(plat)}
                >
                  {!isAll && (
                    <span
                      className="platform-dot"
                      style={{ backgroundColor: meta.color }}
                    />
                  )}
                  <span>{meta.label}</span>
                  {count !== undefined && count > 0 && (
                    <span className="platform-tab-count">{count}</span>
                  )}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      {/* ── 2-Column Master-Detail Catalog Layout ─────────────── */}
      <div className={`meeting-catalog-layout ${selectedMeetingId ? "has-selected-meeting" : ""}`}>
        {/* Left Column: Meeting Cards List */}
        <div className="meeting-cards-list">
          {processing && (
            <div
              id="meetings-processing-status"
              data-testid="meetings-processing-status"
              className="meeting-processing-card"
              role="status"
              aria-live="polite"
              aria-label={`Processing meeting: ${processing.title}`}
            >
              <span className="meeting-processing-dot" aria-hidden="true" />
              <div className="meeting-processing-body">
                <div className="meeting-processing-top-row">
                  <span className="meeting-processing-label">Processing meeting…</span>
                  <span className="meeting-processing-elapsed">{processingElapsed}s</span>
                </div>
                <h3 className="meeting-card-title">{processing.title}</h3>
                <span className="meeting-processing-detail">Transcribing and separating speakers</span>
              </div>
            </div>
          )}
          {!processing && processingError && (
            <div id="meetings-processing-error" data-testid="meetings-processing-error" className="meeting-processing-card meeting-processing-card--error" role="alert">
              <span className="meeting-processing-dot meeting-processing-dot--error" aria-hidden="true" />
              <div className="meeting-processing-body">
                <span className="meeting-processing-label">Meeting processing failed</span>
                <span className="meeting-processing-detail">{processingError}</span>
              </div>
              <button
                id="meetings-dismiss-processing-error-btn"
                data-testid="meetings-dismiss-processing-error-btn"
                type="button"
                className="meeting-processing-dismiss"
                aria-label="Dismiss processing error"
                onClick={() => setProcessingError(null)}
              >
                <IconX size={12} />
              </button>
            </div>
          )}
          {loadingList && meetings.length === 0 ? (
            <div className="meeting-list-empty">Loading recorded meetings...</div>
          ) : meetings.length === 0 && processing ? null : meetings.length === 0 ? (
            <div className="meeting-list-empty">
              <IconVideo size={28} style={{ opacity: 0.3 }} />
              <p>No recorded calls found.</p>
              <span style={{ fontSize: "11px" }}>
                {selectedPlatform !== "All" || searchQuery
                  ? "Try clearing search or platform filters to see other calls."
                  : "When you record or finish calls, they will appear here with the full diarized transcript."}
              </span>
            </div>
          ) : groupByPlatform ? (
            Object.entries(groupedMeetings).map(([plat, groupList]) => {
              const meta = getPlatformMeta(plat);
              const isCollapsed = !!collapsedPlatforms[plat];

              return (
                <div key={plat} className="meeting-platform-group">
                  <div
                    className="meeting-platform-group-header"
                    onClick={() => toggleCollapsePlatform(plat)}
                  >
                    <div className="platform-group-title-wrap">
                      <span className="platform-group-chevron">
                        {isCollapsed ? <IconChevronRight size={13} /> : <IconChevronDown size={13} />}
                      </span>
                      <span
                        className="platform-group-badge"
                        style={{
                          backgroundColor: meta.bg,
                          color: meta.color,
                          borderColor: meta.border,
                        }}
                      >
                        {meta.label}
                      </span>
                    </div>
                    <span className="platform-group-count">
                      {groupList.length} {groupList.length === 1 ? "call" : "calls"}
                    </span>
                  </div>

                  {!isCollapsed && (
                    <div className="platform-group-cards">
                      {groupList.map((m) => {
                        const isSelected = selectedMeetingId === m.id;
                        const d = new Date(m.created_at);
                        const dateStr = `${d.toLocaleDateString(undefined, {
                          month: "short",
                          day: "numeric",
                        })} • ${d.toLocaleTimeString(undefined, {
                          hour: "numeric",
                          minute: "2-digit",
                        })}`;

                        return (
                          <div
                            key={m.id}
                            className={`meeting-list-card ${isSelected ? "selected" : ""}`}
                            onClick={() => selectMeeting(m.id)}
                          >
                            <div className="meeting-card-top-row">
                              <span
                                className="meeting-card-platform-badge"
                                style={{
                                  backgroundColor: meta.bg,
                                  color: meta.color,
                                  borderColor: meta.border,
                                }}
                                onClick={(e) => {
                                  e.stopPropagation();
                                  setSelectedPlatform(m.platform);
                                }}
                                title={`Filter by ${m.platform}`}
                              >
                                {meta.label}
                              </span>
                              <span className="meeting-card-date">{dateStr}</span>
                            </div>

                            <h3 className="meeting-card-title">{m.title}</h3>

                            <div className="meeting-card-bottom-row">
                              <span className="meeting-card-duration">
                                <IconClock size={11} />
                                {formatDurationMs(m.duration_ms)}
                              </span>
                              <span className="meeting-card-speakers-count">
                                <IconUsers size={11} />
                                {m.speaker_count} {m.speaker_count === 1 ? "speaker" : "speakers"}
                              </span>
                              {m.audio_missing && <span className="meeting-card-audio-missing" title="The recording file is gone; the transcript is still here">Audio missing</span>}
                            </div>
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })
          ) : (
            meetings.map((m) => {
              const isSelected = selectedMeetingId === m.id;
              const d = new Date(m.created_at);
              const dateStr = `${d.toLocaleDateString(undefined, {
                month: "short",
                day: "numeric",
              })} • ${d.toLocaleTimeString(undefined, {
                hour: "numeric",
                minute: "2-digit",
              })}`;
              const meta = getPlatformMeta(m.platform);

              return (
                <div
                  key={m.id}
                  className={`meeting-list-card ${isSelected ? "selected" : ""}`}
                  onClick={() => selectMeeting(m.id)}
                >
                  <div className="meeting-card-top-row">
                    <span
                      className="meeting-card-platform-badge"
                      style={{
                        backgroundColor: meta.bg,
                        color: meta.color,
                        borderColor: meta.border,
                      }}
                      onClick={(e) => {
                        e.stopPropagation();
                        setSelectedPlatform(m.platform);
                      }}
                      title={`Filter by ${m.platform}`}
                    >
                      {meta.label}
                    </span>
                    <span className="meeting-card-date">{dateStr}</span>
                  </div>

                  <h3 className="meeting-card-title">{m.title}</h3>

                  <div className="meeting-card-bottom-row">
                    <span className="meeting-card-duration">
                      <IconClock size={11} />
                      {formatDurationMs(m.duration_ms)}
                    </span>
                    <span className="meeting-card-speakers-count">
                      <IconUsers size={11} />
                      {m.speaker_count} {m.speaker_count === 1 ? "speaker" : "speakers"}
                    </span>
                    {m.audio_missing && <span className="meeting-card-audio-missing" title="The recording file is gone; the transcript is still here">Audio missing</span>}
                  </div>
                </div>
              );
            })
          )}
        </div>

        {/* Right Column: Selected Meeting Deep-Dive Detail View */}
        <div className="meeting-detail-card" key={selectedMeeting?.id ?? "none"}>
          {loadingDetail ? (
            <div className="meeting-detail-empty">Loading meeting transcript...</div>
          ) : !selectedMeeting ? (
            <div className="meeting-detail-empty">
              <IconVideo size={36} style={{ opacity: 0.2 }} />
              <h4>Select a meeting to view transcript</h4>
              <p style={{ fontSize: "12px", maxWidth: "260px" }}>
                Review the full speaker-diarized transcript, listen to call audio, and play back isolated voice snippets.
              </p>
            </div>
          ) : (
            <>
              {/* Header */}
              <div className="detail-header">
                <div className="detail-top-row">
                  <button
                    id="meetings-detail-back-btn"
                    data-testid="meetings-detail-back-btn"
                    type="button"
                    className="detail-back-btn"
                    onClick={() => {
                      selectMeeting(null);
                    }}
                    title="Back to all meetings"
                  >
                    <IconChevronDown style={{ transform: "rotate(90deg)" }} size={13} />
                    <span>Meetings</span>
                  </button>

                  <div className="detail-actions-toolbar">
                    <button
                      id="meetings-copy-transcript-btn"
                      data-testid="meetings-copy-transcript-btn"
                      type="button"
                      className="detail-tool-btn"
                      onClick={handleCopyTranscript}
                      title="Copy full meeting transcript to clipboard"
                    >
                      {copiedTranscript ? <IconCheck size={12} /> : <IconCopy size={12} />}
                      <span>{copiedTranscript ? "Copied" : "Copy Transcript"}</span>
                    </button>

                    <button
                      id="meetings-export-markdown-btn"
                      data-testid="meetings-export-markdown-btn"
                      type="button"
                      className="detail-tool-btn"
                      onClick={() => handleExport("markdown")}
                      title="Export transcript as Markdown"
                    >
                      <IconDownloadFile size={13} />
                      <span>Export</span>
                    </button>

                    <button
                      id="meetings-delete-btn"
                      data-testid="meetings-delete-btn"
                      type="button"
                      className="detail-tool-btn danger"
                      onClick={handleDeleteMeeting}
                      title="Delete this meeting recording"
                    >
                      <IconTrash size={13} />
                    </button>
                  </div>
                </div>

                <input
                  aria-label="Meeting title"
                  id="meetings-detail-title-input"
                  data-testid="meetings-detail-title-input"
                  type="text"
                  className="detail-title-input"
                  value={editingTitle}
                  onChange={(e) => setEditingTitle(e.target.value)}
                  onBlur={handleSaveTitle}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleSaveTitle();
                  }}
                  title="Click to rename meeting title"
                />

                <div className="detail-meta-row">
                  <div className="detail-meta-badges">
                    <span
                      className="meeting-card-platform-badge"
                      style={{
                        backgroundColor: getPlatformMeta(selectedMeeting.platform).bg,
                        color: getPlatformMeta(selectedMeeting.platform).color,
                        borderColor: getPlatformMeta(selectedMeeting.platform).border,
                      }}
                    >
                      {getPlatformMeta(selectedMeeting.platform).label}
                    </span>
                    {selectedMeeting.url && (
                      <a
                        href={selectedMeeting.url}
                        target="_blank"
                        rel="noopener noreferrer"
                        className="detail-platform-url-link"
                        title={selectedMeeting.url}
                      >
                        <IconExternalLink size={11} />
                        <span>{selectedMeeting.url.replace(/^https?:\/\//, "").slice(0, 24)}</span>
                      </a>
                    )}
                  </div>
                  <div className="detail-meta-stats">
                    <span>{new Date(selectedMeeting.created_at).toLocaleDateString(undefined, { month: "short", day: "numeric" })} {new Date(selectedMeeting.created_at).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })}</span>
                    <span className="detail-meta-sep">•</span>
                    <span>{formatDurationMs(selectedMeeting.duration_ms)}</span>
                    <span className="detail-meta-sep">•</span>
                    <span className="detail-speaker-meta-pill">
                      <IconUsers size={11} style={{ marginRight: 4 }} />
                      {selectedMeeting.speaker_count} {selectedMeeting.speaker_count === 1 ? "speaker" : "speakers"}
                    </span>
                  </div>
                </div>
              </div>

              {/* Recording file gone: say so; the transcript below is complete. */}
              {(selectedMeeting.audio_missing || audioFailed || (selectedMeeting.clips_missing ?? 0) > 0) && (
                <div id="meeting-audio-missing" data-testid="meeting-audio-missing" className="meeting-audio-missing" role="status">
                  {(selectedMeeting.audio_missing || audioFailed)
                    ? "Audio file missing — the recording was deleted or moved. The transcript is complete."
                    : `${selectedMeeting.clips_missing} speaker clip${selectedMeeting.clips_missing === 1 ? " is" : "s are"} missing — the transcript is complete.`}
                </div>
              )}

              {/* Audio Scrubber Bar (if recording audio exists) */}
              {selectedMeeting.audio_path && !selectedMeeting.audio_missing && !audioFailed && (
                <div className="meeting-audio-player">
                  <button
                    id="meetings-audio-play-btn"
                    data-testid="meetings-audio-play-btn"
                    type="button"
                    className="audio-play-toggle-btn"
                    onClick={handleToggleFullAudio}
                    title={isPlayingAudio ? "Pause Audio" : "Play Recording"}
                  >
                    {isPlayingAudio ? <IconStop size={14} /> : <IconPlay size={14} />}
                  </button>

                  <div className="audio-timeline-wrap">
                    <span className="audio-time-label">
                      {formatSeconds(audioCurrentTime)}
                    </span>
                    <input
                      aria-label="Playback position"
                      id="meetings-audio-scrubber"
                      data-testid="meetings-audio-scrubber"
                      type="range"
                      className="audio-scrubber-slider"
                      min={0}
                      max={audioDuration || Math.round(selectedMeeting.duration_ms / 1000) || 1}
                      step={0.5}
                      value={audioCurrentTime}
                      onChange={(e) => handleScrubAudio(parseFloat(e.target.value))}
                    />
                    <span className="audio-time-label">
                      {formatSeconds(
                        audioDuration || Math.round(selectedMeeting.duration_ms / 1000)
                      )}
                    </span>
                  </div>

                  <button
                    id="meetings-audio-speed-btn"
                    data-testid="meetings-audio-speed-btn"
                    type="button"
                    className="audio-speed-btn"
                    onClick={handleCycleSpeed}
                    title="Change playback speed"
                  >
                    {playbackSpeed}x
                  </button>
                </div>
              )}

              {/* Diarized Conversation Turns */}
              <div className="detail-turns-section">
                <div className="detail-section-heading">
                  <span>
                    <IconUsers size={13} style={{ marginRight: 5, color: "#3b82f6" }} />
                    Meeting Transcript ({selectedMeeting.turns.length} turns)
                  </span>
                </div>

                <div className="turns-conversation-flow">
                  {selectedMeeting.turns.map((t, idx) => {
                    const isYou = t.channel === 0;
                    const isSnippetPlaying = playingSnippetSpeakerId === t.speaker_id;
                    const isRenaming = renamingSpeakerId === t.speaker_id;

                    return (
                      <div
                        key={idx}
                        className={`turn-bubble ${
                          isYou ? "turn-bubble--you" : "turn-bubble--remote"
                        }`}
                      >
                        <div className="turn-header">
                          <div className="turn-speaker-badge-wrap">
                            <div className="turn-speaker-avatar">
                              {isYou ? "YOU" : t.speaker_name.slice(0, 2).toUpperCase()}
                            </div>

                            {isRenaming ? (
                              <input
                                id={`meetings-speaker-rename-input-${idx}`}
                                data-testid={`meetings-speaker-rename-input-${idx}`}
                                type="text"
                                style={{
                                  background: "#000",
                                  border: "1px solid #3b82f6",
                                  color: "#fff",
                                  padding: "2px 6px",
                                  borderRadius: 4,
                                  fontSize: 12,
                                }}
                                aria-label="Speaker name (Enter to save, Escape to cancel)"
                                value={speakerNewName}
                                onChange={(e) => setSpeakerNewName(e.target.value)}
                                onKeyDown={(e) => {
                                  if (e.key === "Enter") handleRenameSpeaker(t.speaker_id);
                                  if (e.key === "Escape") setRenamingSpeakerId(null);
                                }}
                                autoFocus
                              />
                            ) : (
                              <span className="turn-speaker-name">{t.speaker_name}</span>
                            )}

                            <span className="turn-time-stamp">
                              {formatSeconds(Math.round(t.start_ms / 1000))}
                            </span>
                          </div>

                          <div className="turn-actions">
                            {!isYou && t.snippet_path && (
                              <button
                                id={`meetings-play-snippet-btn-${idx}`}
                                data-testid={`meetings-play-snippet-btn-${idx}`}
                                type="button"
                                className={`turn-clip-btn ${
                                  isSnippetPlaying ? "playing" : ""
                                }`}
                                onClick={() => handlePlaySnippet(t.speaker_id, t.snippet_path)}
                                title="Play isolated 3s voice snippet"
                              >
                                {isSnippetPlaying ? (
                                  <IconStop size={11} />
                                ) : (
                                  <IconPlay size={11} />
                                )}
                                <span>{isSnippetPlaying ? "Playing" : "3s Clip"}</span>
                              </button>
                            )}

                            {!isYou && t.candidate_snippets && t.candidate_snippets.length > 1 && (
                              <button
                                id={`meetings-cycle-sample-btn-${idx}`}
                                data-testid={`meetings-cycle-sample-btn-${idx}`}
                                type="button"
                                className="turn-clip-btn cycle-sample-btn"
                                onClick={() => selectedMeeting && handleCycleSnippet(selectedMeeting.id, t.speaker_id)}
                                title="Find more voices: Cycle through alternative clear isolated voice samples for this speaker"
                              >
                                <IconRefresh size={11} />
                                <span>Sample {(t.current_snippet_idx ?? 0) + 1}/{t.candidate_snippets.length}</span>
                              </button>
                            )}

                            {!isYou && (
                              <button
                                id={`meetings-rename-speaker-btn-${idx}`}
                                data-testid={`meetings-rename-speaker-btn-${idx}`}
                                type="button"
                                className="turn-clip-btn"
                                onClick={() => {
                                  if (isRenaming) {
                                    handleRenameSpeaker(t.speaker_id);
                                  } else {
                                    setRenamingSpeakerId(t.speaker_id);
                                    setSpeakerNewName(t.speaker_name);
                                  }
                                }}
                                title="Rename speaker"
                              >
                                {isRenaming ? <IconCheck size={11} /> : <IconEdit size={11} />}
                              </button>
                            )}
                          </div>
                        </div>

                        <p className="turn-text">{t.text}</p>
                      </div>
                    );
                  })}
                </div>
              </div>
            </>
          )}
        </div>
      </div>

      {/* ── Speaker Vault Modal ───────────────────────────────── */}
      <SpeakerVaultModal
        isOpen={isVaultOpen}
        onClose={() => setIsVaultOpen(false)}
        onOpenModelSettings={onOpenModelSettings}
        onSpeakerUpdated={() => {
          if (selectedMeetingId) loadMeetingDetail(selectedMeetingId);
        }}
      />

      {/* ── Post-Meeting Review Modal ──────────────────────────── */}
      <MeetingReviewModal
        meetingId={reviewMeetingId}
        onClose={() => setReviewMeetingId(null)}
        onViewInCatalog={(id) => {
          setReviewMeetingId(null);
          selectMeeting(id);
        }}
      />
    </div>
  );
}
