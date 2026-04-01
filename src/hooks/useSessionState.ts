import { useCallback, useState } from "react";
import type { SessionNotice, SessionState } from "../types/session";

const INITIAL_STATE: SessionState = {
  phase: "idle",
  notice: null,
  lastTranscript: "",
  latestLatency: null,
};

export function useSessionState() {
  const [sessionState, setSessionState] = useState<SessionState>(INITIAL_STATE);

  const setSessionPhase = useCallback((phase: SessionState["phase"]) => {
    setSessionState((prev) => ({ ...prev, phase }));
  }, []);

  const setSessionNotice = useCallback((notice: SessionNotice | null) => {
    setSessionState((prev) => ({ ...prev, notice }));
  }, []);

  const setLastTranscript = useCallback((lastTranscript: string) => {
    setSessionState((prev) => ({ ...prev, lastTranscript }));
  }, []);

  const setLatestLatency = useCallback((latestLatency: number | null) => {
    setSessionState((prev) => ({ ...prev, latestLatency }));
  }, []);

  const patchSessionState = useCallback((patch: Partial<SessionState>) => {
    setSessionState((prev) => ({ ...prev, ...patch }));
  }, []);

  return {
    sessionState,
    setSessionPhase,
    setSessionNotice,
    setLastTranscript,
    setLatestLatency,
    patchSessionState,
  };
}
