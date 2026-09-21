import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { SPEAKER_MODEL_ID } from "../components/settings/types";

/**
 * Whether the speaker recognition model is downloaded. The Speaker Vault
 * (voiceprints, recognising people across meetings) does not work without it.
 * `null` while unknown. Re-checks whenever `recheckKey` changes.
 */
export function useSpeakerModelInstalled(recheckKey?: unknown) {
    const [installed, setInstalled] = useState<boolean | null>(null);

    const refresh = useCallback(async () => {
        try {
            const statuses = await invoke<{ id: string; downloaded: boolean }[]>("get_download_status", {
                modelIds: [SPEAKER_MODEL_ID],
            });
            setInstalled(Boolean(statuses.find((s) => s.id === SPEAKER_MODEL_ID)?.downloaded));
        } catch {
            setInstalled(false);
        }
    }, []);

    useEffect(() => {
        refresh();
    }, [refresh, recheckKey]);

    return { installed, refresh };
}
