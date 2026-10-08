import { expect, test } from "bun:test";
import { DIARIZATION_MODEL_ID, SPEAKER_MODEL_ID, type DownloadableModel } from "../../src/components/settings/types";
import {
    MEETING_SPEAKERS_ID, cancelMeetingSpeakers, deleteMeetingSpeakers, downloadMeetingSpeakers,
    meetingSpeakersInstalled, meetingSpeakersModel, meetingSpeakersProgress,
} from "../../src/utils/meetingSpeakers";

// The two speaker models are one "Meeting speakers" download.
const models = (separator: boolean, recognition: boolean): DownloadableModel[] => [
    { id: DIARIZATION_MODEL_ID, name: "Speaker Separation", type: "Speaker", size: "199 MB", description: "", downloaded: separator, verified: separator },
    { id: SPEAKER_MODEL_ID, name: "Speaker Recognition", type: "Speaker", size: "28 MB", description: "", downloaded: recognition, verified: recognition },
    { id: "whisper-base", name: "Base", type: "Whisper", size: "", description: "", downloaded: true },
];

test("one row: installed only when both parts are, and a half install says what is missing", () => {
    expect(meetingSpeakersModel(models(true, true))).toMatchObject({ id: MEETING_SPEAKERS_ID, name: "Meeting speakers", downloaded: true, verified: true });
    expect(meetingSpeakersInstalled(models(true, true))).toBe(true);
    const half = meetingSpeakersModel(models(true, false));
    expect(half.downloaded).toBe(false);
    expect(half.description).toStartWith("Partly installed: the speaker recognition model is missing.");
    expect(meetingSpeakersModel(models(false, false)).description).not.toContain("Partly");
});

test("Download fetches only the missing parts; Delete removes every installed part", async () => {
    const downloads: string[] = [];
    downloadMeetingSpeakers(models(true, false), (id) => downloads.push(id));
    expect(downloads).toEqual([SPEAKER_MODEL_ID]);
    downloads.length = 0;
    downloadMeetingSpeakers(models(false, false), (id) => downloads.push(id));
    expect(downloads).toEqual([DIARIZATION_MODEL_ID, SPEAKER_MODEL_ID]);

    const deleted: string[] = [];
    await deleteMeetingSpeakers(models(true, true), async (id) => { deleted.push(id); });
    expect(deleted).toEqual([DIARIZATION_MODEL_ID, SPEAKER_MODEL_ID]);
});

test("a failed delete still tries the other part and reports the failure", async () => {
    const deleted: string[] = [];
    const run = deleteMeetingSpeakers(models(true, true), async (id) => {
        if (id === DIARIZATION_MODEL_ID) throw new Error("A recording is in progress");
        deleted.push(id);
    });
    await expect(run).rejects.toThrow("A recording is in progress");
    expect(deleted).toEqual([SPEAKER_MODEL_ID]);
});

test("progress adds the parts' bytes and counts parts as files; cancel stops both", () => {
    expect(meetingSpeakersProgress(models(false, false), {})).toBeUndefined();
    const progress = {
        [DIARIZATION_MODEL_ID]: { bytes: 100, total: 199, status: "downloading" },
        [SPEAKER_MODEL_ID]: { bytes: 28, total: 28, status: "done" },
    };
    expect(meetingSpeakersProgress(models(false, false), progress)).toMatchObject({ bytes: 128, total: 227, status: "downloading", current_file: 1, total_files: 2 });
    const cancelled: string[] = [];
    cancelMeetingSpeakers((id) => cancelled.push(id), progress);
    expect(cancelled).toEqual([DIARIZATION_MODEL_ID, SPEAKER_MODEL_ID]);
});
