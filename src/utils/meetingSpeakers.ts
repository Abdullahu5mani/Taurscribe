// "Meeting speakers": the speaker separation and speaker recognition models,
// shown and managed as one download in Settings → Models. Meetings need both
// (who spoke when, and who that is), so they are installed and removed together.
import { DIARIZATION_MODEL_ID, SPEAKER_MODEL_ID, type DownloadableModel, type DownloadProgress } from "../components/settings/types";

export const MEETING_SPEAKERS_ID = "meeting-speakers";
export const MEETING_SPEAKER_PARTS = [DIARIZATION_MODEL_ID, SPEAKER_MODEL_ID] as const;

const PART_NAMES: Record<string, string> = {
    [DIARIZATION_MODEL_ID]: "speaker separation",
    [SPEAKER_MODEL_ID]: "speaker recognition",
};

const parts = (models: DownloadableModel[]) =>
    MEETING_SPEAKER_PARTS.map((id) => models.find((m) => m.id === id)).filter((m): m is DownloadableModel => !!m);

/** True when both models are installed. */
export function meetingSpeakersInstalled(models: DownloadableModel[]): boolean {
    const found = parts(models);
    return found.length === MEETING_SPEAKER_PARTS.length && found.every((m) => m.downloaded);
}

/** The single row shown in place of the two models. */
export function meetingSpeakersModel(models: DownloadableModel[]): DownloadableModel {
    const found = parts(models);
    const missing = found.filter((m) => !m.downloaded);
    const installed = meetingSpeakersInstalled(models);
    const partly = !installed && missing.length < found.length;
    return {
        id: MEETING_SPEAKERS_ID,
        name: "Meeting speakers",
        type: "Speaker",
        size: "227 MB",
        description:
            (partly ? `Partly installed: the ${missing.map((m) => PART_NAMES[m.id]).join(" and ")} model is missing. ` : "") +
            "Tells the people in a meeting apart (up to 8) and recognises them by voice across meetings, for the Speaker Vault. " +
            "Two models: Nemotron 3 speaker separation (OpenMDW licence) and CAM++ voiceprints.",
        downloaded: installed,
        verified: installed && found.every((m) => m.verified),
    };
}

/**
 * Progress of the bundle: bytes of every part that is downloading, verifying
 * or deleting, added up; "file n of N" counts the parts.
 */
export function meetingSpeakersProgress(
    models: DownloadableModel[],
    progress: Record<string, DownloadProgress>,
): DownloadProgress | undefined {
    const active = MEETING_SPEAKER_PARTS.map((id, index) => ({ id, index, p: progress[id] })).filter((x) => x.p);
    if (active.length === 0) return undefined;
    const lead = active.find((x) => x.p!.status !== "done") ?? active[0];
    const total = active.reduce((sum, x) => sum + (x.p!.total || 0), 0);
    const bytes = active.reduce((sum, x) => sum + (x.p!.bytes || 0), 0);
    const toInstall = parts(models).filter((m) => !m.downloaded).length || MEETING_SPEAKER_PARTS.length;
    return {
        ...lead.p!,
        bytes,
        total,
        current_file: Math.min(active.indexOf(lead) + 1, toInstall),
        total_files: toInstall,
    };
}

/** Downloads whichever parts are missing. */
export function downloadMeetingSpeakers(models: DownloadableModel[], onDownload: (id: string, name: string) => void) {
    for (const m of parts(models)) if (!m.downloaded) onDownload(m.id, m.name);
}

/** Deletes every installed part; the first failure is reported. */
export async function deleteMeetingSpeakers(models: DownloadableModel[], onDelete: (id: string, name: string) => Promise<void>) {
    let failure: unknown = null;
    for (const m of parts(models)) {
        if (!m.downloaded) continue;
        try {
            await onDelete(m.id, m.name);
        } catch (error) {
            failure ??= error;
        }
    }
    if (failure) throw failure;
}

export function cancelMeetingSpeakers(onCancel: (id: string) => void, progress: Record<string, DownloadProgress>) {
    for (const id of MEETING_SPEAKER_PARTS) if (progress[id]) onCancel(id);
}
