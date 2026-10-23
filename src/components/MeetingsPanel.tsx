import { claimAudioPlayback, releaseAudioPlayback } from "../utils/audioPlayback";
import { serialTasks } from "../utils/serialTasks";
import { useState, useEffect, useRef, useMemo } from "react";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { SpeakerSuggestions } from "./SpeakerSuggestions";
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
} from "./Icons";
import type { MeetingInfo } from "./MeetingHeaderPill";
import {
  MeetingSummaryRecord,
  MeetingDetailRecord,
  PlatformCount,
} from "../types/meetings";
import { MODELS } from "./settings/types";
import { MeetingTags } from "./MeetingTags";
import { MeetingHistoryControls } from "./MeetingHistoryControls";
import { SpeakerPreviewModal } from "./SpeakerPreviewModal";
import { SpeakerVaultModal } from "./SpeakerVaultModal";
import { MeetingReviewModal } from "./MeetingReviewModal";
import { levelToPercent } from "../utils/audioLevel";
import { MeetingRecordingImport, recordingDate } from "./MeetingRecordingImport";
import { conversationBlocks, spokenAt, speakerCountHint, interpolateWords } from "../utils/meetingTimeline";
import "./MeetingsPanel.css";
import "./MeetingWorkspace.css";
import "./MeetingStudio.css";

interface MeetingsPanelProps {
  modelId?: string | null;
  useGpu?: boolean;
  activeMeeting: MeetingInfo | null;
  isRecording: boolean;
  isDualChannelRecording: boolean;
  dualLevels: { mic: number; system: number };
  onStartDualRecording: () => void;
  onStopRecording: () => void;
  onOpenSettings?: () => void;
  /** Opens Settings → Models (speaker recognition model for the Speaker Vault). */
  onOpenModelSettings?: () => void;
  /** Shows the in-person recording section (InPersonPanel). */
  onRecordInPerson?: () => void;
  /** The in-person recording section, shown at the top while open. */
  inPerson?: React.ReactNode;
  isVisible?: boolean;
  importBlocked?: boolean;
  hasDiarizationModel?: boolean;
  onImportProcessingChange?: (busy: boolean) => void;
}



const PLATFORM_META: Record<string, { label: string; color: string; bg: string; border: string }> = {
  "Google Meet": { label: "Google Meet", color: "#10b981", bg: "rgba(16, 185, 129, 0.14)", border: "rgba(16, 185, 129, 0.3)" },
  "Zoom": { label: "Zoom", color: "#3b82f6", bg: "rgba(59, 130, 246, 0.14)", border: "rgba(59, 130, 246, 0.3)" },
  "Microsoft Teams": { label: "Teams", color: "#8b5cf6", bg: "rgba(139, 92, 246, 0.14)", border: "rgba(139, 92, 246, 0.3)" },
  "Slack": { label: "Slack", color: "#f59e0b", bg: "rgba(245, 158, 11, 0.14)", border: "rgba(245, 158, 11, 0.3)" },
  "Webex": { label: "Webex", color: "#06b6d4", bg: "rgba(6, 182, 212, 0.14)", border: "rgba(6, 182, 212, 0.3)" },
  "Discord": { label: "Discord", color: "#6366f1", bg: "rgba(99, 102, 241, 0.14)", border: "rgba(99, 102, 241, 0.3)" },
  "Direct Audio": { label: "Direct Audio", color: "#ec4899", bg: "rgba(236, 72, 153, 0.14)", border: "rgba(236, 72, 153, 0.3)" },
  "In person": { label: "In person", color: "#14b8a6", bg: "rgba(20, 184, 166, 0.14)", border: "rgba(20, 184, 166, 0.3)" },
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

/** A safe download name for a meeting's notes ("Budget: Q3/Q4" → "Budget_Q3_Q4_transcript.md"). */
export function exportFileName(title: string, format: "markdown" | "text" | "json"): string {
  const base = title.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, " ").trim().replace(/\s+/g, "_") || "meeting";
  return `${base}_transcript.${format === "markdown" ? "md" : format === "json" ? "json" : "txt"}`;
}

/** Speaker colours cycle through six tones; "You" is always tone 0. */
const SPEAKER_TONE_COUNT = 6;

export function speakerInitials(name: string): string {
  const trimmed = name.trim();
  const words = trimmed.split(/\s+/).filter(Boolean);
  if (words.length === 0) return "?";
  const numbered = trimmed.match(/^speaker\s*(\d+)$/i);
  if (numbered) return `S${numbered[1]}`;
  // By code point, not UTF-16 unit: indexing split emoji and other characters
  // outside the BMP in half and showed a broken glyph.
  const chars = (word: string) => Array.from(word);
  const first = chars(words[0]);
  return (words.length === 1 ? first.slice(0, 2).join("") : first[0] + chars(words[words.length - 1])[0]).toUpperCase();
}

/** "Today", "Yesterday", "This week", or the month, for the library's date headers. */
export function dayBucket(iso: string, now: Date = new Date()): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "Earlier";
  const startOfDay = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(d)) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return "This week";
  return d.toLocaleDateString(undefined, { month: "long", year: d.getFullYear() === now.getFullYear() ? undefined : "numeric" });
}

/** Wraps case-insensitive matches of `query` in <mark>; returns the plain text when there is no query. */
function highlightMatches(text: string, query: string): React.ReactNode {
  const q = query.trim();
  if (!q) return text;
  const lower = text.toLowerCase(), needle = q.toLowerCase();
  const out: React.ReactNode[] = [];
  let from = 0;
  for (let at = lower.indexOf(needle); at !== -1; at = lower.indexOf(needle, from)) {
    if (at > from) out.push(text.slice(from, at));
    out.push(<mark key={at}>{text.slice(at, at + needle.length)}</mark>);
    from = at + needle.length;
  }
  if (from < text.length) out.push(text.slice(from));
  return out;
}

export function MeetingsPanel({
  modelId, useGpu,
  activeMeeting,
  isRecording,
  isDualChannelRecording,
  dualLevels,
  onStartDualRecording,
  onStopRecording,
  onOpenModelSettings,
  onRecordInPerson,
  inPerson,
  isVisible = true,
  importBlocked = false,
  hasDiarizationModel = true,
  onImportProcessingChange,
}: MeetingsPanelProps) {
  const visibleRef = useRef(isVisible);
  visibleRef.current = isVisible;
  // Meetings Catalog
  const [meetings, setMeetings] = useState<MeetingSummaryRecord[]>([]);
  const meetingDropTargetRef = useRef<HTMLDivElement>(null);
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
  const selectedAudioPathRef = useRef<string | null>(null);

  const timelineRefs = useRef<Map<number, HTMLDivElement>>(new Map());
  const manualScrollUntil = useRef(0);
  const scrolledBlock = useRef(-1);
  const [speakerHint, setSpeakerHint] = useState("");
  const [assignBlock, setAssignBlock] = useState<number | null>(null);
  const [assignPerson, setAssignPerson] = useState("");
  const [assignName, setAssignName] = useState("");
  const [assignAll, setAssignAll] = useState(false);
  const transcriptScroll = useRef<HTMLDivElement>(null);
  const scrollToPassage = (index: number) => {
    const pane = transcriptScroll.current, node = timelineRefs.current.get(index);
    if (pane && node) pane.scrollTo({ top: pane.scrollTop + node.getBoundingClientRect().top - pane.getBoundingClientRect().top - pane.clientHeight / 2 + node.clientHeight / 2, behavior: "smooth" });
  };
  const [detailView, setDetailView] = useState<"transcript" | "speakers" | "tools">("transcript");
  const [confirmPipeline, setConfirmPipeline] = useState(false);
  const [confirmReseparate, setConfirmReseparate] = useState(false);
  const [speakerBusy, setSpeakerBusy] = useState(false);
  const speakerMutation = useRef(false);
  const blocks = useMemo(() => conversationBlocks(selectedMeeting?.turns ?? []), [selectedMeeting?.turns]);
  const meetingSpeakers = useMemo(() => Array.from(new Map((selectedMeeting?.turns ?? []).map(turn => [turn.speaker_id, turn])).values()), [selectedMeeting?.turns]);
  const [transcriptQuery, setTranscriptQuery] = useState("");
  // Colour and talk time per speaker, in order of first appearance.
  const speakerStats = useMemo(() => {
    const stats = new Map<string, { tone: number; talkMs: number; share: number }>();
    let next = 0, total = 0;
    for (const turn of selectedMeeting?.turns ?? []) {
      let entry = stats.get(turn.speaker_id);
      if (!entry) {
        // Tone 0 is the local mic's; everyone else cycles through 1..5.
        entry = { tone: turn.channel === 0 ? 0 : 1 + (next++ % (SPEAKER_TONE_COUNT - 1)), talkMs: 0, share: 0 };
        stats.set(turn.speaker_id, entry);
      }
      const ms = Math.max(0, turn.end_ms - turn.start_ms);
      entry.talkMs += ms;
      total += ms;
    }
    for (const entry of stats.values()) entry.share = total > 0 ? entry.talkMs / total : 0;
    return stats;
  }, [selectedMeeting?.turns]);
  const toneOf = (speakerId: string) => speakerStats.get(speakerId)?.tone ?? 0;
  const visibleBlocks = useMemo(() => {
    const q = transcriptQuery.trim().toLowerCase();
    const all = blocks.map((block, index) => ({ block, index }));
    return q ? all.filter(({ block }) => block.text.toLowerCase().includes(q) || block.speaker_name.toLowerCase().includes(q)) : all;
  }, [blocks, transcriptQuery]);
  useEffect(() => { setTranscriptQuery(""); }, [selectedMeetingId]);
  const activeBlock = blocks.findIndex(t => spokenAt(t.intervals, audioCurrentTime));
  const turnsOfMeeting = selectedMeeting?.turns;
  // Word times are estimates inside each source turn, so a long merged passage
  // is followed word by word while it plays.
  const blockWords = useMemo(
    () => blocks.map(b => b.sourceIndices.flatMap(i => {
      const turn = turnsOfMeeting?.[i];
      return turn ? interpolateWords(turn.text, turn.start_ms, turn.end_ms) : [];
    })),
    [blocks, turnsOfMeeting],
  );
  const activeWord = (() => {
    if (activeBlock < 0) return -1;
    const words = blockWords[activeBlock] ?? [];
    const ms = audioCurrentTime * 1000;
    let found = -1;
    for (let i = 0; i < words.length; i++) {
      if (words[i].start_ms <= ms) found = i; else break;
    }
    return found;
  })();
  useEffect(() => {
    if (!isPlayingAudio || typeof requestAnimationFrame === "undefined") return;
    let frame = 0;
    const tick = () => {
      const audio = fullAudioRef.current;
      if (audio) setAudioCurrentTime(audio.currentTime);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [isPlayingAudio]);
  useEffect(() => {
    if (isPlayingAudio && activeBlock >= 0 && scrolledBlock.current !== activeBlock && Date.now() >= manualScrollUntil.current) {
      scrolledBlock.current = activeBlock;
      scrollToPassage(activeBlock);
    }
  }, [activeBlock, isPlayingAudio, audioCurrentTime]);

  // A passage can be taller than the pane. While it plays, keep the word being
  // spoken in a comfortable band instead of centring the whole passage.
  useEffect(() => {
    if (!isPlayingAudio || activeWord < 0 || Date.now() < manualScrollUntil.current) return;
    const pane = transcriptScroll.current;
    const word = pane?.querySelector<HTMLElement>("[data-word-active=\"true\"]");
    if (!pane || !word) return;
    const p = pane.getBoundingClientRect(), w = word.getBoundingClientRect();
    const from = p.top + p.height * 0.25, to = p.top + p.height * 0.62;
    if (w.top < from || w.bottom > to) {
      pane.scrollTo({ top: pane.scrollTop + (w.top - p.top) - p.height * 0.38, behavior: "smooth" });
    }
  }, [activeWord, activeBlock, isPlayingAudio]);

  // Speaker Snippet Preview Playback
  const [previewSpeaker, setPreviewSpeaker] = useState<{ id: string; name: string; path?: string | null } | null>(null);
  const [playingSnippetSpeakerId, setPlayingSnippetSpeakerId] = useState<string | null>(null);
  const snippetAudioRef = useRef<HTMLAudioElement | null>(null);

  // Modals & Renaming
  const [isVaultOpen, setIsVaultOpen] = useState(false);
  const [reviewMeetingId, setReviewMeetingId] = useState<number | null>(null);
  // A stopped call being transcribed + diarized (between the backend's
  // meeting-processing-started and meeting-processing-finished events).
  const [processing, setProcessing] = useState<{ title: string; platform: string; startedAt: number; stage?: string; job_id?: string } | null>(null);
  const [importPreview, setImportPreview] = useState<{ title: string; model_id?: string; paragraphs: { start_ms: number; end_ms: number; text: string }[]; completed_ms: number; total_ms: number } | null>(null);
  const [showImportPreview, setShowImportPreview] = useState(false);
  const previewOpenRef = useRef(false);
  const openImportPreview = () => { stopFullAudio(); stopSnippetAudio(); previewOpenRef.current = true; setShowImportPreview(true); };
  const [processingError, setProcessingError] = useState<string | null>(null);
  const [processingStopped, setProcessingStopped] = useState(false);
  const [stoppingProcessing, setStoppingProcessing] = useState(false);
  const stopProcessing = async () => {
    setStoppingProcessing(true);
    try { if (!await invoke<boolean>("cancel_meeting_processing", {jobId:processing?.job_id ?? null})) setStoppingProcessing(false); }
    catch (error) { setStoppingProcessing(false); setProcessingError(String(error)); }
  };
  const [processingElapsed, setProcessingElapsed] = useState(0);
  const [editingTitle, setEditingTitle] = useState("");
  const titleDraft = useRef<string | null>(null);
  const titleWrites = useRef(serialTasks());
  const titleRequests = useRef(new Map<number, { revision: number; title: string }>());
  const savedTitles = useRef(new Map<number, { revision: number; title: string }>());
  const titleRevision = useRef(0);
  const [renamingSpeakerId, setRenamingSpeakerId] = useState<string | null>(null);
  // Speaker ids like "speaker_remote_1" repeat across meetings: close any open
  // rename field when another meeting is opened.
  useEffect(() => {
    setAssignBlock(null); setRenamingSpeakerId(null); setSpeakerHint(""); setConfirmReseparate(false); setConfirmPipeline(false); setDetailView("transcript");
  }, [selectedMeetingId]);
  const [speakerNewName, setSpeakerNewName] = useState("");
  const [copiedTranscript, setCopiedTranscript] = useState(false);
  const [exportedNotes, setExportedNotes] = useState(false);
  /** Delete was clicked once for this meeting and awaits confirmation. */
  const [confirmDeleteMeetingId, setConfirmDeleteMeetingId] = useState<number | null>(null);
  /** A failed copy / export / delete, shown under the toolbar. */
  const [toolbarError, setToolbarError] = useState("");

  const selectMeeting = (id: number | null) => {
    previewOpenRef.current = false; setShowImportPreview(false);
    if (selectedMeetingIdRef.current !== id) {
      detailRequestRef.current += 1;
      selectedMeetingIdRef.current = id;
      setSelectedMeeting(null);
      titleDraft.current = null;
      setEditingTitle("");
      setLoadingDetail(false);
      setConfirmDeleteMeetingId(null);
      setToolbarError("");
      stopFullAudio();
      stopSnippetAudio();
      setPreviewSpeaker(null);
      manualScrollUntil.current = 0; scrolledBlock.current = -1;
      setAudioCurrentTime(0);
      setAudioDuration(0);
      selectedAudioPathRef.current = null;
    }
    setSelectedMeetingId(id);
  };

  // Fetch list of meetings and platform counts
  const [historyPages, setHistoryPages] = useState(1);
  const historyPagesRef = useRef(historyPages);
  historyPagesRef.current = historyPages;
  const loadMeetings = async () => {
    const request = ++listRequestRef.current;
    setLoadingList(true);
    try {
      const [list, counts] = await Promise.all([
        invoke<MeetingSummaryRecord[]>("list_meetings", {
          search: searchQueryRef.current.trim() || null,
          category: null,
          platform: selectedPlatformRef.current === "All" ? null : selectedPlatformRef.current,
          limit: historyPagesRef.current * 100,
          offset: 0,
        }),
        invoke<PlatformCount[]>("get_meeting_platform_counts").catch(() => []),
      ]);
      if (request !== listRequestRef.current) return;
      setMeetings(list);
      setPlatformCounts(counts || []);

      // Auto-select first meeting on wider screens if none selected or if previously selected was deleted
      if (list.length > 0) {
        if (!selectedMeetingIdRef.current || ((searchQueryRef.current.trim() || selectedPlatformRef.current !== "All") && !list.some(m => m.id === selectedMeetingIdRef.current))) {
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
    const savedBeforeRead = savedTitles.current.get(id);
    setLoadingDetail(true);
    stopFullAudio();
    stopSnippetAudio();
    try {
      const detail = await invoke<MeetingDetailRecord>("get_meeting_detail", {
        meetingId: id,
      });
      if (request !== detailRequestRef.current || selectedMeetingIdRef.current !== id) return;
      // A read may have captured the old row while a title write was pending.
      const savedDuringRead = savedTitles.current.get(id);
      if (savedDuringRead && savedDuringRead !== savedBeforeRead) detail.title = savedDuringRead.title;
      const audioPath = detail.audio_path ?? null;
      if (selectedAudioPathRef.current !== audioPath) {
        setAudioCurrentTime(0);
        setAudioDuration(0);
      }
      selectedAudioPathRef.current = audioPath;
      setSelectedMeeting(detail);
      setAudioFailed(false);
      setEditingTitle(titleDraft.current ?? titleRequests.current.get(id)?.title ?? detail.title);
    } catch (err) {
      if (request === detailRequestRef.current) console.error("Failed to load meeting detail:", err);
    } finally {
      if (request === detailRequestRef.current) setLoadingDetail(false);
    }
  };

  useEffect(() => {
    loadMeetings();
  }, [searchQuery, selectedPlatform, historyPages]);

  useEffect(() => {
    if (selectedMeetingId !== null) {
      loadMeetingDetail(selectedMeetingId);
    }
  }, [selectedMeetingId]);

  // Processing indicator lifecycle
  useEffect(() => {
    let disposed = false; let receivedLifecycleEvent = false;
    let pendingStage: string | undefined;
    let pendingPreview: typeof importPreview | undefined;
    const unlisteners: Promise<() => void>[] = [
      listen<{ title: string; platform: string; job_id?: string }>("meeting-processing-started", (event) => {
        if (disposed) return;
        receivedLifecycleEvent = true; setProcessingStopped(false); setStoppingProcessing(false);
        setProcessingError(null);
        setImportPreview(null); previewOpenRef.current = false; setShowImportPreview(false);
        setProcessingElapsed(0);
        setProcessing({
          title: event.payload?.title || "Meeting",
          platform: event.payload?.platform || "",
          startedAt: Date.now(), job_id:event.payload?.job_id,
        });
      }),
      listen<{ stage: string }>("meeting-processing-progress", event => {
        if (disposed) return;
        pendingStage = event.payload.stage;
        setProcessing(previous => previous ? { ...previous, stage: event.payload.stage } : previous);
      }),
      listen<{ title: string; model_id?: string; paragraphs: { start_ms: number; end_ms: number; text: string }[]; completed_ms: number; total_ms: number }>("meeting-processing-preview", event => {
        if (disposed) return;
        pendingPreview = event.payload; setImportPreview(event.payload);
      }),
      listen<{ meeting_id: number | null; error: string | null; cancelled?: boolean }>("meeting-processing-finished", (event) => {
        if (disposed) return;
        receivedLifecycleEvent = true; setProcessing(null); setStoppingProcessing(false);
        const stopped=!!event.payload?.cancelled || !!event.payload?.error?.includes("Meeting processing stopped");
        const savedWithGap = !!event.payload?.meeting_id && !!event.payload?.error?.startsWith("Meeting saved");
        setProcessingStopped(stopped);
        if (stopped) {setProcessingError(null);} else if (event.payload?.error) {
          setProcessingError(event.payload.error);
        } else if (!event.payload?.meeting_id) {
          setProcessingError("Nothing was saved: no speech was transcribed from the recording.");
        } else {
          setProcessingError(null);
        }
        loadMeetings();
        const savedId = event.payload?.meeting_id;
        if (savedId && (!event.payload?.error || savedWithGap) && previewOpenRef.current) {
          selectMeeting(savedId);
        }
        if (savedId && (!event.payload?.error || savedWithGap) && savedId === selectedMeetingIdRef.current) {
          void loadMeetingDetail(savedId);
        }
      }),
    ];
    void Promise.all(unlisteners).then(() => invoke<any>("get_meeting_import_status")).then(status => {
      if (disposed || receivedLifecycleEvent || !status) return;
      setImportPreview(pendingPreview ?? status.preview); const stopped=!!status.error?.includes("Meeting processing stopped");setProcessingStopped(stopped);setProcessingError(stopped ? null : status.error);
      if (status.running) setProcessing({title:status.title,platform:status.platform,startedAt:status.startedAt,stage:pendingStage ?? status.stage,job_id:status.job_id});
    }).catch(() => {});
    return () => {
      disposed = true;
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
    let disposed = false;
    listen<{ meeting_id: number; imported?: boolean }>("meeting-processing-complete", (event) => {
      const id = event.payload?.meeting_id;
      if (id) {
        // Imported meetings finish quietly; live meeting review stays in its visible view.
        if (!event.payload.imported && visibleRef.current) setReviewMeetingId(id);
        loadMeetings();
        if (id === selectedMeetingIdRef.current) void loadMeetingDetail(id);
      }
    }).then((u) => {
      if (disposed) u();
      else unlisten = u;
    });

    return () => {
      disposed = true;
      if (unlisten) unlisten();
      stopFullAudio();
      stopSnippetAudio();
    };
  }, []);

  useEffect(() => {
    if (!isVisible) { stopFullAudio(); stopSnippetAudio(); }
  }, [isVisible]);

  // ── Full Audio Player Controls ──────────────────────────────────
  const stopFullAudio = () => {
    if (fullAudioRef.current) {
      releaseAudioPlayback(fullAudioRef.current);
      fullAudioRef.current = null;
    }
    setIsPlayingAudio(false);
  };

  const handleToggleFullAudio = (seekTo?: number, forcePlay = false) => {
    if (!selectedMeeting?.audio_path) return;

    if (isPlayingAudio && !forcePlay) {
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
        audio.onplay = () => { if (fullAudioRef.current === audio) setIsPlayingAudio(true); };
        audio.playbackRate = playbackSpeed;
        audio.currentTime = seekTo ?? audioCurrentTime;

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
      if (seekTo !== undefined) handleScrubAudio(seekTo);
      claimAudioPlayback(audio, () => setIsPlayingAudio(false));
      audio.play().then(() => {
        if (fullAudioRef.current === audio && !audio.paused) setIsPlayingAudio(true);
      }).catch((error) => {
        // Pausing or hiding the window cancels a pending play request normally.
        if (error?.name === "AbortError") return;
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
    const target = Math.max(0, targetSec);
    setAudioCurrentTime(target);
    manualScrollUntil.current = 0;
    const next = blocks.findIndex(t => t.end_ms / 1000 > target);
    const index = next < 0 ? blocks.length - 1 : next;
    scrolledBlock.current = index;
    scrollToPassage(index);
    if (fullAudioRef.current) {
      fullAudioRef.current.currentTime = target;
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
      releaseAudioPlayback(snippetAudioRef.current);
      snippetAudioRef.current = null;
    }
    setPlayingSnippetSpeakerId(null);
  };

  const handlePlaySnippet = (speakerId: string, snippetPath?: string | null, replace = false) => {
    const clipKey = `${speakerId}:${snippetPath ?? ""}`;
    if (!replace && playingSnippetSpeakerId === clipKey) {
      stopSnippetAudio();
      return;
    }
    stopSnippetAudio();
    if (!snippetPath) return;

    try {
      const assetUrl = convertFileSrc(snippetPath);
      const audio = new Audio(assetUrl);
      snippetAudioRef.current = audio;
      setPlayingSnippetSpeakerId(clipKey);

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

      claimAudioPlayback(audio, () => { if (snippetAudioRef.current === audio) { snippetAudioRef.current = null; setPlayingSnippetSpeakerId(null); } });
      audio.play().catch((error) => {
        console.error("Snippet playback failed:", error);
        if (snippetAudioRef.current === audio) stopSnippetAudio();
      });
    } catch (e) {
      console.error("Snippet play error:", e);
    }
  };

  const changeSpeakers = async (command: string, args: Record<string, unknown>) => {
    if (!selectedMeeting || speakerMutation.current) return;
    const id = selectedMeeting.id;
    if (command === "reseparate_meeting_speakers") {
      try { args.expectedSpeakers = speakerCountHint(speakerHint); }
      catch (error) { setToolbarError(String(error)); return; }
    }
    speakerMutation.current = true; setSpeakerBusy(true);
    try {
      const rerun = command === "reseparate_meeting_speakers" || command === "reprocess_meeting";
      if (rerun) { onImportProcessingChange?.(true); stopFullAudio(); }
      try { await invoke(command, { meetingId: id, ...args }); }
      finally { if (rerun) onImportProcessingChange?.(false); }
      if (selectedMeetingIdRef.current === id) await loadMeetingDetail(id);
      void loadMeetings();
      if (selectedMeetingIdRef.current === id) setToolbarError("");
    } catch (error) {
      if (selectedMeetingIdRef.current === id) setToolbarError(String(error));
    } finally { speakerMutation.current = false; setSpeakerBusy(false); }
  };

  const assignPassage = async () => {
    if (!selectedMeeting || assignBlock === null || speakerMutation.current) return;
    const block = blocks[assignBlock];
    if (!block) return;
    const indices = assignAll ? selectedMeeting.turns.flatMap((turn, index) => turn.speaker_id === block.speaker_id ? [index] : []) : block.sourceIndices;
    const ids = indices.map(index => selectedMeeting.turn_ids?.[index]);
    if (ids.some(id => id === undefined)) { setToolbarError("Reopen this meeting before assigning a speaker."); return; }
    const meetingId = selectedMeeting.id;
    speakerMutation.current = true; setSpeakerBusy(true);
    try {
      await invoke("assign_meeting_turns", { meetingId, turnIds: ids, speakerId: assignPerson || null, newName: assignPerson ? null : assignName.trim() });
      if (selectedMeetingIdRef.current === meetingId) { setAssignBlock(null); setToolbarError(""); await loadMeetingDetail(meetingId); }
      void loadMeetings();
    } catch (error) { if (selectedMeetingIdRef.current === meetingId) setToolbarError(String(error)); }
    finally { speakerMutation.current = false; setSpeakerBusy(false); }
  };

  // ── Detail Updates ─────────────────────────────────────────────
  const handleSaveTitle = async () => {
    if (!selectedMeeting || !editingTitle.trim()) {
      return;
    }
    const meetingId = selectedMeeting.id;
    const newTitle = editingTitle.trim();
    const previous = titleRequests.current.get(meetingId);
    if (previous?.title === newTitle) return;
    if (!previous && newTitle === selectedMeeting.title) {
      titleDraft.current = null;
      setEditingTitle(newTitle);
      return;
    }
    const revision = ++titleRevision.current;
    titleRequests.current.set(meetingId, { revision, title: newTitle });
    try {
      await titleWrites.current(() => invoke("update_meeting_title", { meetingId, title: newTitle }));
      savedTitles.current.set(meetingId, { revision, title: newTitle });
      if (titleRequests.current.get(meetingId)?.revision !== revision) return;
      titleRequests.current.delete(meetingId);
      if (selectedMeetingIdRef.current === meetingId) {
        if (titleDraft.current?.trim() === newTitle) {
          titleDraft.current = null;
          setEditingTitle(newTitle);
        }
        setSelectedMeeting((prev) => (prev?.id === meetingId ? { ...prev, title: newTitle } : prev));
        setToolbarError("");
      }
      loadMeetings();
    } catch (err) {
      console.error("Failed to update meeting title:", err);
      if (titleRequests.current.get(meetingId)?.revision === revision) {
        titleRequests.current.delete(meetingId);
        if (selectedMeetingIdRef.current === meetingId) setToolbarError(`Couldn't save the title: ${err}`);
      }
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

  /** First click asks for confirmation; the second deletes. */
  const handleDeleteMeeting = async () => {
    if (!selectedMeeting || selectedMeeting.id !== selectedMeetingIdRef.current) return;
    const meetingId = selectedMeeting.id;
    if (confirmDeleteMeetingId !== meetingId) {
      setConfirmDeleteMeetingId(meetingId);
      return;
    }
    setConfirmDeleteMeetingId(null);
    try {
      await invoke("delete_meeting", { meetingId });
      setToolbarError("");
      if (selectedMeetingIdRef.current === meetingId) selectMeeting(null);
      loadMeetings();
    } catch (err) {
      console.error("Failed to delete meeting:", err);
      setToolbarError(`Couldn't delete this meeting: ${err}`);
    }
  };

  /** Saves the notes to a file the user picks (Markdown, text or JSON). */
  const handleExport = async (format: "markdown" | "text" | "json") => {
    if (!selectedMeeting) return;
    const meetingId = selectedMeeting.id;
    const fileName = exportFileName(selectedMeeting.title, format);
    const extension = fileName.slice(fileName.lastIndexOf(".") + 1);
    try {
      const path = await saveDialog({
        defaultPath: fileName,
        filters: [{ name: format === "markdown" ? "Markdown" : format === "json" ? "JSON" : "Text", extensions: [extension] }],
      });
      if (!path) return; // cancelled
      await invoke("save_meeting_notes", { meetingId, format, path });
      setToolbarError("");
      setExportedNotes(true);
      setTimeout(() => setExportedNotes(false), 2000);
    } catch (err) {
      console.error("Failed to export meeting notes:", err);
      setToolbarError(`Couldn't export the notes: ${err}`);
    }
  };

  const handleCopyTranscript = async () => {
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
    try {
      await navigator.clipboard.writeText(text);
    } catch (err) {
      setToolbarError(`Couldn't copy the transcript: ${err}`);
      return;
    }
    setToolbarError("");
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
    <div ref={meetingDropTargetRef} className="meetings-panel meetings-panel--workspace" data-testid="meetings-panel">
      {inPerson}
      <div className="meeting-workspace-header">
        <div className="meeting-workspace-title">
          <h1>Meetings</h1>
          <p>{platformCounts.reduce((sum, platform) => sum + platform.count, 0) || meetings.length} saved · {formatDurationMs(meetings.reduce((sum, m) => sum + m.duration_ms, 0))} listed</p>
        </div>
        <div className="meeting-workspace-actions">
          {onRecordInPerson && !inPerson && !isRecording && (
            <button
              id="meetings-record-in-person-btn"
              data-testid="meetings-record-in-person-btn"
              type="button"
              className="meeting-vault-btn meeting-record-in-person-btn"
              onClick={onRecordInPerson}
              title="Record a meeting in the room on this computer's microphone"
            >
              <IconRecord size={12} />
              <span className="vault-btn-text">Record in person</span>
            </button>
          )}

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
      <MeetingRecordingImport
        compact isVisible={isVisible} modelId={modelId} useGpu={useGpu}
        dropTargetRef={meetingDropTargetRef}
        blocked={isRecording || !!processing || importBlocked || isVaultOpen || reviewMeetingId !== null}
        hasDiarizationModel={hasDiarizationModel}
        onProcessingChange={onImportProcessingChange}
        onComplete={id => { setSelectedPlatform("All"); setSearchQuery(""); selectMeeting(id); void loadMeetings(); }}
      />
        </div>
      </div>

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
                {activeMeeting.app_name}
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
            <h4 className="meeting-meters__title">Audio levels</h4>
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
      {previewSpeaker && selectedMeeting && <SpeakerPreviewModal key={`${selectedMeeting.id}-${previewSpeaker.id}`} meetingId={selectedMeeting.id}
        speakerId={previewSpeaker.id} speakerName={previewSpeaker.name} initialPath={previewSpeaker.path} onClose={() => setPreviewSpeaker(null)} />}



      {/* ── 2-Column Master-Detail Catalog Layout ─────────────── */}
      <div className={`meeting-catalog-layout ${selectedMeetingId || showImportPreview ? "has-selected-meeting" : ""}`}>
        <aside className="meeting-library" aria-label="Meeting library">
<div className="meeting-filter-bar">
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
          <div className="meeting-library-filters">
            <select id="meetings-platform-filter" data-testid="meetings-platform-filter" aria-label="Filter meetings by platform" value={selectedPlatform} onChange={e => setSelectedPlatform(e.target.value)}>
              {availablePlatforms.map(platform => <option key={platform} value={platform}>{platform === "All" ? "All sources" : getPlatformMeta(platform).label}</option>)}
            </select>
            <select id="meetings-list-order" data-testid="meetings-list-order" aria-label="Meeting list order" value={groupByPlatform ? "platform" : "recent"} onChange={e => setGroupByPlatform(e.target.value === "platform")}>
              <option value="recent">Recent first</option><option value="platform">By source</option>
            </select>
          </div>
        </div>
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
                <span className="meeting-processing-detail">{processing.stage || "Preparing meeting processing"}</span>
                {!!importPreview && importPreview.total_ms > 0 && (
                  <div id="meetings-processing-progress" data-testid="meetings-processing-progress" className="meeting-processing-progress" role="progressbar" aria-label="Transcription progress"
                    aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.min(100, Math.round(importPreview.completed_ms / importPreview.total_ms * 100))}>
                    <span style={{ width: `${Math.min(100, importPreview.completed_ms / importPreview.total_ms * 100)}%` }} />
                  </div>
                )}
                {!!importPreview?.paragraphs.length && <button id="meetings-processing-read-transcript-btn" data-testid="meetings-processing-read-transcript-btn" className="detail-tool-btn" type="button" onClick={openImportPreview}>Read transcript · speaker labels pending</button>}
                <button id="meeting-processing-stop" data-testid="meeting-processing-stop" className="detail-tool-btn" type="button" disabled={stoppingProcessing || processing.stage?.startsWith("Saving")} onClick={() => void stopProcessing()}>{stoppingProcessing ? "Stopping…" : "Stop processing"}</button>
              </div>
            </div>
          )}
          {!processing && processingStopped && <div id="meeting-processing-stopped" data-testid="meeting-processing-stopped" className="meeting-processing-card" role="status">
            <div className="meeting-processing-body"><span>Meeting processing stopped</span><span className="meeting-processing-detail">Saved meetings and original recordings are unchanged.</span>
              {!!importPreview?.paragraphs.length && <button id="meetings-stopped-read-transcript-btn" data-testid="meetings-stopped-read-transcript-btn" className="detail-tool-btn" type="button" onClick={openImportPreview}>Read partial transcript</button>}
              <button id="meetings-stopped-dismiss-btn" data-testid="meetings-stopped-dismiss-btn" className="detail-tool-btn" type="button" onClick={() => setProcessingStopped(false)}>Dismiss</button>
            </div>
          </div>}
          {!processing && processingError && (
            <div id="meetings-processing-error" data-testid="meetings-processing-error" className="meeting-processing-card meeting-processing-card--error" role="alert">
              <span className="meeting-processing-dot meeting-processing-dot--error" aria-hidden="true" />
              <div className="meeting-processing-body">
                <span className="meeting-processing-label">{processingError.startsWith("Meeting saved") ? "Meeting saved" : "Meeting processing failed"}</span>
                <span className="meeting-processing-detail">{processingError}</span>
                {!!importPreview?.paragraphs.length && <button id="meetings-error-read-transcript-btn" data-testid="meetings-error-read-transcript-btn" className="detail-tool-btn" type="button" onClick={openImportPreview}>Read partial transcript</button>}
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
              <p>{selectedPlatform !== "All" || searchQuery ? "No meetings match." : "No meetings yet."}</p>
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
                          <div id={`meetings-card-${m.id}`} data-testid={`meetings-card-${m.id}`}
                            key={m.id}
                            className={`meeting-list-card ${isSelected ? "selected" : ""}`}
                            role="button" tabIndex={0} aria-label={`Open meeting ${m.title}`}
                            onKeyDown={e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); selectMeeting(m.id); } }}
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
            meetings.flatMap((m, position) => {
              const isSelected = selectedMeetingId === m.id;
              const bucket = dayBucket(m.created_at);
              const showBucket = position === 0 || dayBucket(meetings[position - 1].created_at) !== bucket;
              const d = new Date(m.created_at);
              const dateStr = `${d.toLocaleDateString(undefined, {
                month: "short",
                day: "numeric",
              })} • ${d.toLocaleTimeString(undefined, {
                hour: "numeric",
                minute: "2-digit",
              })}`;
              const meta = getPlatformMeta(m.platform);

              return [
                showBucket ? <div key={`bucket-${bucket}-${m.id}`} className="meeting-date-header">{bucket}</div> : null,
                <div id={`meetings-card-${m.id}`} data-testid={`meetings-card-${m.id}`}
                  key={m.id}
                  className={`meeting-list-card ${isSelected ? "selected" : ""}`}
                  role="button" tabIndex={0} aria-label={`Open meeting ${m.title}`}
                  onKeyDown={e => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); selectMeeting(m.id); } }}
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
                </div>,
              ];
            })
          )}
          {meetings.length >= historyPages * 100 && <button id="meetings-load-more-btn" data-testid="meetings-load-more-btn" type="button" className="meeting-load-more" disabled={loadingList} onClick={() => setHistoryPages(pages => pages + 1)}>{loadingList ? "Loading…" : "Load more meetings"}</button>}
        </div>

        <MeetingHistoryControls count={platformCounts.reduce((sum, platform) => sum + platform.count, 0)} onSelect={selectMeeting} />
        </aside>

        {/* Right Column: Selected Meeting Deep-Dive Detail View */}
        <div className="meeting-detail-card"
          onPointerDown={() => { manualScrollUntil.current = Date.now() + 5000; scrolledBlock.current = -1; }}
          onWheel={() => { manualScrollUntil.current = Date.now() + 5000; scrolledBlock.current = -1; }}
          onTouchMove={() => { manualScrollUntil.current = Date.now() + 5000; scrolledBlock.current = -1; }}
          onKeyDown={e => { if (["PageUp", "PageDown", "ArrowUp", "ArrowDown", "Home", "End"].includes(e.key)) { manualScrollUntil.current = Date.now() + 5000; scrolledBlock.current = -1; } }} key={selectedMeeting?.id ?? "none"}>
          {showImportPreview && importPreview ? (
            <>
              <div className="detail-header meeting-draft-header">
                <button id="meetings-draft-back-btn" data-testid="meetings-draft-back-btn" className="detail-tool-btn" type="button" onClick={() => { previewOpenRef.current = false; setShowImportPreview(false); }}>Back to meetings</button>
                <h3>{importPreview.title}</h3>
                <div className="meeting-transcription-model">Transcribed with <strong>{MODELS.find(model => model.id === importPreview.model_id)?.name || importPreview.model_id || "Selected speech model"}</strong></div>
                <p id="meetings-draft-status" data-testid="meetings-draft-status" role="status">{processingStopped ? "Partial draft · stopped · not saved" : processingError?.startsWith("Meeting saved") ? "Saved · some recorded audio is missing" : processingError ? "Partial draft · processing failed · not saved" : "Draft transcript · speaker labels pending"}</p>
                <p>{Math.round(importPreview.completed_ms / 1000)} / {Math.round(importPreview.total_ms / 1000)} seconds transcribed</p>
              </div>
              <div className="meeting-detail-scroll" data-testid="meeting-import-preview">
                {importPreview.paragraphs.map((paragraph, index) => <div className="meeting-draft-passage" key={`${paragraph.start_ms}-${index}`}>
                  <span>{Math.floor(paragraph.start_ms / 60000)}:{String(Math.floor(paragraph.start_ms / 1000) % 60).padStart(2, "0")}</span>
                  <p>{paragraph.text}</p>
                </div>)}
              </div>
            </>
          ) : loadingDetail ? (
            <div className="meeting-detail-empty">Loading meeting transcript...</div>
          ) : !selectedMeeting ? (
            <div className="meeting-detail-empty">
              <IconVideo size={36} style={{ opacity: 0.2 }} />
              <h4>Select a meeting to view transcript</h4>
              <p style={{ fontSize: "12px", maxWidth: "260px" }}>
                Review the full speaker-diarized transcript, listen to call audio, and compare speaker previews.
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
                      {exportedNotes ? <IconCheck size={12} /> : <IconDownloadFile size={13} />}
                      <span>{exportedNotes ? "Exported" : "Export"}</span>
                    </button>


                  </div>
                </div>
                {toolbarError && (
                  <p id="meetings-toolbar-error" data-testid="meetings-toolbar-error" className="detail-toolbar-error" role="alert">
                    {toolbarError}
                  </p>
                )}

                <input
                  aria-label="Meeting title"
                  id="meetings-detail-title-input"
                  data-testid="meetings-detail-title-input"
                  type="text"
                  className="detail-title-input"
                  value={editingTitle}
                  onChange={(e) => { titleDraft.current = e.target.value; setEditingTitle(e.target.value); }}
                  onBlur={handleSaveTitle}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") handleSaveTitle();
                  }}
                  title="Click to rename meeting title"
                />

                <MeetingTags key={selectedMeeting.id} meetingId={selectedMeeting.id} onChanged={() => { void loadMeetings(); }} />
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
                {meetingSpeakers.filter(person => person.speaker_id !== "background").length > 1 && speakerStats.size > 0 && (
                  <button id="meeting-talkshare" data-testid="meeting-talkshare" type="button" className="meeting-talkshare" onClick={() => setDetailView("speakers")}
                    aria-label="Talk time by speaker. Open speakers.">
                    <span className="meeting-talkshare-bar" aria-hidden="true">
                      {meetingSpeakers.filter(person => person.speaker_id !== "background").map(person => {
                        const stat = speakerStats.get(person.speaker_id);
                        return stat && stat.share > 0 ? <span key={person.speaker_id} data-tone={stat.tone} style={{ flexGrow: stat.share }} title={`${person.speaker_name} · ${Math.round(stat.share * 100)}%`} /> : null;
                      })}
                    </span>
                    <span className="meeting-talkshare-legend">
                      {meetingSpeakers.filter(person => person.speaker_id !== "background").slice(0, 4).map(person => {
                        const stat = speakerStats.get(person.speaker_id);
                        return <span key={person.speaker_id} data-tone={stat?.tone ?? 0}><i />{person.speaker_name} {Math.round((stat?.share ?? 0) * 100)}%</span>;
                      })}
                    </span>
                  </button>
                )}
              </div>

              {/* Recording file gone: say so; the transcript below is complete. */}
              {(selectedMeeting.audio_missing || audioFailed || (selectedMeeting.clips_missing ?? 0) > 0) && (
                <div id="meeting-audio-missing" data-testid="meeting-audio-missing" className="meeting-audio-missing" role="status">
                  {(selectedMeeting.audio_missing || audioFailed)
                    ? "Audio file missing — the recording was deleted or moved. The transcript is complete."
                    : `${selectedMeeting.clips_missing} speaker clip${selectedMeeting.clips_missing === 1 ? " is" : "s are"} missing — the transcript is complete.`}
                </div>
              )}

                <div className="meeting-transcription-model" data-testid="meeting-transcription-model">Transcribed with <strong>{MODELS.find(model => model.id === selectedMeeting.transcription_model)?.name || selectedMeeting.transcription_model || "Unknown (older meeting)"}</strong></div>

              {/* Audio Scrubber Bar (if recording audio exists) */}
              {selectedMeeting.audio_path && !selectedMeeting.audio_missing && !audioFailed && (
                <div className="meeting-audio-player">
                  <button
                    id="meetings-audio-play-btn"
                    data-testid="meetings-audio-play-btn"
                    type="button"
                    className="audio-play-toggle-btn"
                    onClick={() => handleToggleFullAudio()}
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

              <div id="meetings-detail-tablist" data-testid="meetings-detail-tablist" className="meeting-detail-tabs" role="tablist" aria-label="Meeting view" onKeyDown={e => {
                if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(e.key)) return;
                e.preventDefault();
                const views = ["transcript", "speakers", "tools"] as const;
                const next = e.key === "Home" ? 0 : e.key === "End" ? 2 : (views.indexOf(detailView) + (e.key === "ArrowRight" ? 1 : 2)) % 3;
                setDetailView(views[next]);
                if (typeof document !== "undefined") document.getElementById(`meetings-tab-${views[next]}`)?.focus();
              }}>
                {(["transcript", "speakers", "tools"] as const).map(view => <button type="button" role="tab" key={view} id={`meetings-tab-${view}`} data-testid={`meetings-tab-${view}`} aria-controls={`meetings-panel-${view}`} aria-selected={detailView === view} tabIndex={detailView === view ? 0 : -1} onClick={() => setDetailView(view)}>
                  {view === "transcript" ? "Transcript" : view === "speakers" ? `Speakers · ${meetingSpeakers.filter(person => person.speaker_id !== "background").length}` : "Tools"}
                </button>)}
                {detailView === "transcript" && blocks.length > 0 && (
                  <label className="meeting-transcript-search">
                    <IconSearch size={13} />
                    <input id="meetings-transcript-search" data-testid="meetings-transcript-search" type="search" aria-label="Search this transcript" placeholder="Search transcript"
                      value={transcriptQuery} onChange={e => setTranscriptQuery(e.target.value)}
                      onKeyDown={e => { if (e.key === "Escape") setTranscriptQuery(""); }} />
                    {transcriptQuery.trim() && <span aria-live="polite">{visibleBlocks.length} of {blocks.length}</span>}
                  </label>
                )}
              </div>
              <div className="meeting-detail-scroll" ref={transcriptScroll}>
              {/* Diarized Conversation Turns */}
              <section id="meetings-panel-transcript" data-testid="meetings-panel-transcript" className="detail-turns-section" role="tabpanel" aria-labelledby="meetings-tab-transcript" hidden={detailView !== "transcript"}>
                <div className="turns-conversation-flow">
                  {transcriptQuery.trim() && visibleBlocks.length === 0 && (
                    <p id="meetings-transcript-nomatch" data-testid="meetings-transcript-nomatch" className="meeting-transcript-nomatch" role="status">No passage mentions “{transcriptQuery.trim()}”.</p>
                  )}
                  {visibleBlocks.map(({ block: t, index: idx }) => {
                    const isYou = t.channel === 0;

                    return (
                      <div
                        key={idx}
                        ref={node => { if (node) timelineRefs.current.set(idx, node); else timelineRefs.current.delete(idx); }}
                        data-testid={`meetings-turn-${idx}`}
                        aria-current={spokenAt(t.intervals, audioCurrentTime) ? "true" : undefined}
                        data-tone={toneOf(t.speaker_id)}
                        className={`turn-bubble ${spokenAt(t.intervals, audioCurrentTime) ? "turn-bubble--spoken " : ""}${
                          isYou ? "turn-bubble--you" : "turn-bubble--remote"
                        }`}
                      >
                        <div className="turn-header">
                          <div className="turn-speaker-badge-wrap">
                            <div className="turn-speaker-avatar">
                              {isYou ? "YOU" : speakerInitials(t.speaker_name)}
                            </div>

                            <button id={`meetings-turn-speaker-${idx}`} data-testid={`meetings-turn-speaker-${idx}`} type="button" className="turn-speaker-link" onClick={() => setDetailView("speakers")} aria-label={`Manage speaker ${t.speaker_name}`}>{t.speaker_name}</button>
                            <button id={`meetings-turn-play-${idx}`} data-testid={`meetings-turn-play-${idx}`} type="button" className="turn-time-stamp" aria-label={`Play from ${formatSeconds(t.start_ms / 1000)}`} disabled={!selectedMeeting.audio_path || !!selectedMeeting.audio_missing || audioFailed} onClick={() => handleToggleFullAudio(t.start_ms / 1000, true)}>{formatSeconds(Math.round(t.start_ms / 1000))}</button>
                          </div>


                        </div>

                        <button id={`meetings-turn-assign-${idx}`} data-testid={`meetings-turn-assign-${idx}`} type="button" className="turn-clip-btn turn-assign-btn" aria-label={`Assign speaker for passage at ${formatSeconds(t.start_ms / 1000)}`} disabled={speakerBusy} onClick={() => { setAssignBlock(idx); setAssignPerson(""); setAssignName(""); setAssignAll(false); }}>Assign speaker</button>
                        {assignBlock === idx && <form className="turn-assignment" aria-label="Assign passage speaker" onSubmit={e => { e.preventDefault(); void assignPassage(); }}>
                          <label>Person<select id={`meetings-assign-person-${idx}`} data-testid={`meetings-assign-person-${idx}`} aria-label="Person" value={assignPerson} onChange={e => setAssignPerson(e.target.value)} disabled={speakerBusy}>
                            <option value="">New person…</option>
                            {meetingSpeakers.filter(person => person.speaker_id !== "background").map(person => <option key={person.speaker_id} value={person.speaker_id}>{person.speaker_name}</option>)}
                          </select></label>
                          {!assignPerson && <label>Name<input id={`meetings-assign-name-${idx}`} data-testid={`meetings-assign-name-${idx}`} aria-label="Name" autoFocus value={assignName} maxLength={200} onChange={e => setAssignName(e.target.value)} placeholder="Person’s name" disabled={speakerBusy} /></label>}
                          <label><input id={`meetings-assign-all-${idx}`} data-testid={`meetings-assign-all-${idx}`} aria-label={`Apply to every passage labeled ${t.speaker_name}`} type="checkbox" checked={assignAll} onChange={e => setAssignAll(e.target.checked)} disabled={speakerBusy} />Apply to every passage currently labeled {t.speaker_name}</label>
                          <p>Otherwise, only this passage changes. Voice samples can be added in Speaker Vault.</p>
                          {!!selectedMeeting.merges?.length && <p>Saving a correction clears earlier merge undo history.</p>}
                          <button id={`meetings-assign-save-${idx}`} data-testid={`meetings-assign-save-${idx}`} className="turn-clip-btn" type="submit" disabled={speakerBusy || (!assignPerson && !assignName.trim())}>{speakerBusy ? "Saving…" : "Save speaker"}</button>
                          <button id={`meetings-assign-cancel-${idx}`} data-testid={`meetings-assign-cancel-${idx}`} className="turn-clip-btn" type="button" disabled={speakerBusy} onClick={() => setAssignBlock(null)}>Cancel</button>
                        </form>}
                        <p id={`meetings-turn-text-${idx}`} data-testid={`meetings-turn-text-${idx}`} className="turn-text" role={selectedMeeting.audio_path && !audioFailed ? "button" : undefined}
                          tabIndex={selectedMeeting.audio_path && !audioFailed ? 0 : undefined}
                          aria-label={`Jump to ${formatSeconds(t.start_ms / 1000)}: ${t.text}`}
                          onClick={() => { if (selectedMeeting.audio_path && !audioFailed) handleToggleFullAudio(t.start_ms / 1000, true); }}
                          onKeyDown={e => { if (selectedMeeting.audio_path && !audioFailed && (e.key === "Enter" || e.key === " ")) { e.preventDefault(); handleToggleFullAudio(t.start_ms / 1000, true); } }}>{
                          idx === activeBlock && isPlayingAudio && !transcriptQuery.trim() && (blockWords[idx]?.length ?? 0) > 0
                            ? blockWords[idx].map((w, wi) => (
                                <span key={wi} className={`turn-word${wi < activeWord ? " turn-word--past" : ""}${wi === activeWord ? " turn-word--active" : ""}`}
                                  data-word-active={wi === activeWord ? "true" : undefined}
                                  onClick={e => { e.stopPropagation(); handleToggleFullAudio(w.start_ms / 1000, true); }}>{w.text}{" "}</span>
                              ))
                            : highlightMatches(t.text, transcriptQuery)
                        }</p>
                      </div>
                    );
                  })}
                </div>
              </section>
              <section id="meetings-panel-speakers" data-testid="meetings-panel-speakers" className="meeting-speakers-panel" role="tabpanel" aria-labelledby="meetings-tab-speakers" hidden={detailView !== "speakers"}>
                <div className="meeting-panel-intro"><p>Play a sample, rename a speaker, or merge voices that belong to the same person.</p></div>
              {/* "Speaker 2 sounds like Dana — is it?" */}
              <SpeakerSuggestions
                meetingId={selectedMeeting.id}
                onLabelled={() => { loadMeetingDetail(selectedMeeting.id); loadMeetings(); }}
              />

                {selectedMeeting.merges?.[0] && (
                  <button id="meetings-undo-merge-btn" data-testid="meetings-undo-merge-btn" type="button" className="turn-clip-btn" disabled={speakerBusy}
                    onClick={() => changeSpeakers("undo_meeting_speaker_merge", { mergeId: selectedMeeting.merges![0].id })}>
                    Undo merge: {selectedMeeting.merges[0].from_speaker_name} → {selectedMeeting.merges[0].into_speaker_name}
                  </button>
                )}

                <div className="meeting-speaker-cards">
                  {meetingSpeakers.map((person, idx) => {
                    const t = selectedMeeting.turns.find(turn => turn.speaker_id === person.speaker_id && turn.snippet_path) ?? person;
                    const isYou = t.channel === 0;
                    const isSnippetPlaying = playingSnippetSpeakerId === `${t.speaker_id}:${t.snippet_path ?? ""}`;
                    const isRenaming = renamingSpeakerId === t.speaker_id;
                    const stat = speakerStats.get(t.speaker_id);
                    return <article className="meeting-speaker-card" key={t.speaker_id} data-tone={stat?.tone ?? 0}>
                      <div className="meeting-speaker-card-heading"><div className="turn-speaker-avatar">{isYou ? "YOU" : speakerInitials(t.speaker_name)}</div>
                            {isRenaming ? (
                              <input
                                id={`meetings-speaker-rename-input-${idx}`}
                                data-testid={`meetings-speaker-rename-input-${idx}`}
                                type="text"

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
                              <strong className="turn-speaker-name">{t.speaker_name}</strong>
                            )}


                        <span>{selectedMeeting.turns.filter(turn => turn.speaker_id === t.speaker_id).length} segments{stat ? ` · ${formatDurationMs(stat.talkMs)}` : ""}</span>
                      </div>
                      {stat && <div className="meeting-speaker-share" aria-label={`${Math.round(stat.share * 100)}% of talk time`}><span style={{ width: `${Math.max(2, stat.share * 100)}%` }} /></div>}
                          <div className="meeting-speaker-card-actions">
                            {!isYou && t.snippet_path && (
                              <button
                                id={`meetings-play-snippet-btn-${idx}`}
                                data-testid={`meetings-play-snippet-btn-${idx}`}
                                type="button"
                                className={`turn-clip-btn ${
                                  isSnippetPlaying ? "playing" : ""
                                }`}
                                onClick={() => handlePlaySnippet(t.speaker_id, t.snippet_path)}
                                title="Play a 3-second speaker preview; overlapping voices may also be audible"
                              >
                                {isSnippetPlaying ? (
                                  <IconStop size={11} />
                                ) : (
                                  <IconPlay size={11} />
                                )}
                                <span>{isSnippetPlaying ? "Stop sample" : "Play sample"}</span>
                              </button>
                            )}


                            {!isYou && t.speaker_id !== "background" && <button id={`meetings-compare-samples-${t.speaker_id}`} data-testid={`meetings-compare-samples-${t.speaker_id}`} type="button" className="turn-clip-btn" aria-label={`Compare voice samples for ${t.speaker_name}`}
                              onClick={() => { stopSnippetAudio(); stopFullAudio(); setPreviewSpeaker({ id: t.speaker_id, name: t.speaker_name, path: t.snippet_path }); }}>Compare samples</button>}

                            <select id={`meetings-merge-select-${t.speaker_id}`} data-testid={`meetings-merge-select-${t.speaker_id}`} aria-label={`Merge ${t.speaker_name} into`} className="turn-merge-select"
                              value="" disabled={speakerBusy}
                              onChange={e => { if (e.target.value) void changeSpeakers("merge_meeting_speakers", { fromSpeakerId: t.speaker_id, intoSpeakerId: e.target.value }); }}>
                              <option value="">Merge into…</option>
                              {t.speaker_id !== "background" && <option value="background">Background noise</option>}
                              {meetingSpeakers
                                .filter(other => other.speaker_id !== t.speaker_id && other.speaker_id !== "background")
                                .map(other => <option key={other.speaker_id} value={other.speaker_id}>{other.speaker_name}</option>)}
                            </select>

                            {!isYou && t.speaker_id !== "background" && (
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
                                {isRenaming ? <IconCheck size={11} /> : <IconEdit size={11} />}<span>{isRenaming ? "Save name" : "Rename"}</span>
                              </button>
                            )}

                          </div>
                    </article>;
                  })}
                </div>
              </section>
              <section id="meetings-panel-tools" data-testid="meetings-panel-tools" className="meeting-tools-panel" role="tabpanel" aria-labelledby="meetings-tab-tools" hidden={detailView !== "tools"}>

                <div className="meeting-maintenance-card meeting-danger-zone"><h3>Delete meeting</h3><p>Permanently remove this meeting and its recording.</p><div className="meeting-speaker-card-actions">                    {confirmDeleteMeetingId === selectedMeeting.id ? (
                      <>
                        <button
                          id="meetings-delete-confirm-btn"
                          data-testid="meetings-delete-confirm-btn"
                          type="button"
                          className="detail-tool-btn danger detail-tool-btn--confirm"
                          onClick={handleDeleteMeeting}
                          title="Delete this meeting and its recording permanently"
                        >
                          <IconTrash size={13} />
                          <span>Delete meeting?</span>
                        </button>
                        <button
                          id="meetings-delete-cancel-btn"
                          data-testid="meetings-delete-cancel-btn"
                          type="button"
                          className="detail-tool-btn"
                          onClick={() => setConfirmDeleteMeetingId(null)}
                        >
                          <span>Cancel</span>
                        </button>
                      </>
                    ) : (
                      <button
                        id="meetings-delete-btn"
                        data-testid="meetings-delete-btn"
                        type="button"
                        className="detail-tool-btn danger"
                        onClick={handleDeleteMeeting}
                        title="Delete this meeting recording"
                      >
                        <IconTrash size={13} /><span>Delete meeting</span>
                      </button>
                    )}
                </div></div>
                <div className="meeting-maintenance-card">
                  <h3>Reprocess recording</h3>
                  <p>Use the original recording to rebuild the transcript and speaker labels.</p>
                <div className="meeting-speaker-tools">
                  <label>About how many people? <input id="meetings-speaker-hint-input" data-testid="meetings-speaker-hint-input" type="number" min="1" max="32" step="1" value={speakerHint} aria-label="Speaker count hint" disabled={speakerBusy}
                    onChange={e => setSpeakerHint(e.target.value)} /></label>
                  <button id="meetings-reseparate-btn" data-testid="meetings-reseparate-btn" type="button" className="turn-clip-btn" disabled={speakerBusy || importBlocked || isRecording || !selectedMeeting.audio_path || selectedMeeting.audio_missing}
                    onClick={() => { if (!confirmReseparate) { setConfirmReseparate(true); return; } setConfirmReseparate(false); void changeSpeakers("reseparate_meeting_speakers", { expectedSpeakers: speakerHint ? Number(speakerHint) : null, modelId, useGpu }); }}>
                    {speakerBusy ? "Working…" : confirmReseparate ? "Confirm re-separation" : "Re-separate speakers"}
                  </button>
                  <button id="meetings-reprocess-btn" data-testid="meetings-reprocess-btn" type="button" className="turn-clip-btn" disabled={speakerBusy || importBlocked || isRecording || !selectedMeeting.audio_path || selectedMeeting.audio_missing}
                    onClick={() => { if (!confirmPipeline) { setConfirmPipeline(true); return; } setConfirmPipeline(false); void changeSpeakers("reprocess_meeting", { expectedSpeakers: speakerHint ? Number(speakerHint) : null, modelId, useGpu }); }}>
                    {confirmPipeline ? "Confirm full reprocessing" : "Clean audio and rerun pipeline"}
                  </button>
                  {confirmPipeline && <><span>Rebuilds the transcript from cleaned audio. Separate mic/caller tracks allow echo cleanup; mixed recordings get noise cleanup. Manual turn labels and merge undo are replaced after success. Original audio, notes and tags are kept.</span><button id="meetings-reprocess-cancel-btn" data-testid="meetings-reprocess-cancel-btn" type="button" className="turn-clip-btn" onClick={() => setConfirmPipeline(false)}>Cancel</button></>}
                  {confirmReseparate && <><span>Rebuilds turns and transcript; manual labels and merge undo are replaced after success. Notes and original recording are kept.</span><button id="meetings-reseparate-cancel-btn" data-testid="meetings-reseparate-cancel-btn" type="button" className="turn-clip-btn" onClick={() => setConfirmReseparate(false)}>Cancel</button></>}
                </div>
                </div>
              {!!selectedMeeting.recording_sources?.length && (
                <details className="meeting-import-sources" data-testid="meeting-recording-sources">
                  <summary>{selectedMeeting.recording_sources.length} imported recording{selectedMeeting.recording_sources.length === 1 ? "" : "s"} · original timeline</summary>
                  <ol>{selectedMeeting.recording_sources.map((source, index) => <li key={`${source.path}-${index}`}>
                    <button id={`meetings-source-play-${index}`} data-testid={`meetings-source-play-${index}`} type="button" disabled={!selectedMeeting.audio_path || selectedMeeting.audio_missing || audioFailed}
                      onClick={() => { handleToggleFullAudio(source.offset_ms / 1000, true); }}>
                      {formatDurationMs(source.offset_ms)} · {source.name} · {recordingDate(source.recorded_start)}
                      {source.recorded_end ? ` → ${recordingDate(source.recorded_end)}` : ""}
                    </button>
                    {source.timestamp_source && <p>{source.timestamp_source}</p>}
                  </li>)}</ol>
                  <p>Playback joins the files in your chosen order. Original timestamps retain gaps and overlaps.</p>
                </details>
              )}


              </section>
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
          if (selectedMeetingId === id) void loadMeetingDetail(id);
          void loadMeetings();
        }}
      />
    </div>
  );
}
