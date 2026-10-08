interface Props {
    personId: string;
    personName: string;
    /** "find": speakers in past meetings who may be this person. "samples": their stored voice samples. */
    mode: "find" | "samples";
    /** Something changed in the vault (a sample added or removed). */
    onChanged?: () => void;
    onClose: () => void;
}

const formatDate = (iso: string) => {
    const d = new Date(iso);
    return Number.isNaN(d.getTime()) ? "" : d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
};
const seconds = (ms: number) => `${Math.max(1, Math.round(ms / 1000))}s of speech`;

