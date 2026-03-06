/**
 * Custom SVG icon components for Taurscribe.
 * All icons are 24×24 viewBox, monochrome, and use currentColor by default
 * so they automatically inherit the parent's CSS color.
 *
 * Usage:  <IconTrash className="my-class" style={{ color: '#ef4444' }} />
 */

import React from "react";

interface IconProps extends React.SVGProps<SVGSVGElement> {
    size?: number | string;
}

const defaultProps = (size: number | string = 16): React.SVGProps<SVGSVGElement> => ({
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    xmlns: "http://www.w3.org/2000/svg",
});

// ── Core Controls ─────────────────────────────────────────────────────

export const IconRecord = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} {...props}>
        <circle cx="12" cy="12" r="8" fill="currentColor" />
    </svg>
);

export const IconStop = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} {...props}>
        <rect x="5" y="5" width="14" height="14" rx="2" fill="currentColor" />
    </svg>
);

export const IconProcessing = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} {...props}>
        <circle cx="6" cy="12" r="2.5" fill="currentColor" />
        <circle cx="12" cy="12" r="2.5" fill="currentColor" />
        <circle cx="18" cy="12" r="2.5" fill="currentColor" />
    </svg>
);

export const IconDownload = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
        <polyline points="7 10 12 15 17 10" />
        <line x1="12" y1="15" x2="12" y2="3" />
    </svg>
);

export const IconDownloadOff = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M21 15v4a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-4" />
        <polyline points="7 10 12 15 17 10" />
        <line x1="12" y1="15" x2="12" y2="3" />
        <line x1="22" y1="2" x2="2" y2="22" />
    </svg>
);

export const IconTrash = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polyline points="3 6 5 6 21 6" />
        <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
        <line x1="10" y1="11" x2="10" y2="17" />
        <line x1="14" y1="11" x2="14" y2="17" />
    </svg>
);

export const IconRetry = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M12 4A8 8 0 1 1 4 12" />
        <polyline points="1 15 4 12 7 15" />
        <line x1="9" y1="9" x2="15" y2="15" />
        <line x1="15" y1="9" x2="9" y2="15" />
    </svg>
);

export const IconX = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <line x1="6" y1="6" x2="18" y2="18" />
        <line x1="18" y1="6" x2="6" y2="18" />
    </svg>
);

export const IconCheck = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polyline points="6 13 10 17 18 7" />
    </svg>
);

// ── Status ────────────────────────────────────────────────────────────

export const IconShieldCheck = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
        <polyline points="9 12 11 14 15 10" />
    </svg>
);

export const IconWarning = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" />
        <line x1="12" y1="9" x2="12" y2="13" />
        <line x1="12" y1="17" x2="12.01" y2="17" />
    </svg>
);

export const IconBolt = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} {...props}>
        <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" fill="currentColor" />
    </svg>
);

export const IconCpu = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <rect x="5" y="5" width="14" height="14" rx="2" />
        <line x1="8" y1="2" x2="8" y2="5" /><line x1="12" y1="2" x2="12" y2="5" /><line x1="16" y1="2" x2="16" y2="5" />
        <line x1="8" y1="19" x2="8" y2="22" /><line x1="12" y1="19" x2="12" y2="22" /><line x1="16" y1="19" x2="16" y2="22" />
        <line x1="2" y1="8" x2="5" y2="8" /><line x1="2" y1="12" x2="5" y2="12" /><line x1="2" y1="16" x2="5" y2="16" />
        <line x1="19" y1="8" x2="22" y2="8" /><line x1="19" y1="12" x2="22" y2="12" /><line x1="19" y1="16" x2="22" y2="16" />
    </svg>
);

export const IconMic = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <rect x="9" y="2" width="6" height="12" rx="3" />
        <path d="M19 10v2a7 7 0 0 1-14 0v-2" />
        <line x1="12" y1="19" x2="12" y2="22" />
        <line x1="8" y1="22" x2="16" y2="22" />
    </svg>
);

// ── Volume ────────────────────────────────────────────────────────────

export const IconVolumeHigh = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
        <path d="M14 10a3 3 0 0 1 0 4" />
        <path d="M16 7a6 6 0 0 1 0 10" />
        <path d="M18 4a9 9 0 0 1 0 16" />
    </svg>
);

export const IconVolumeLow = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
        <path d="M15.54 8.46a5 5 0 0 1 0 7.07" />
    </svg>
);

export const IconVolumeMuted = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" />
        <line x1="23" y1="9" x2="17" y2="15" />
        <line x1="17" y1="9" x2="23" y2="15" />
    </svg>
);

// ── Tone Style Tiles ──────────────────────────────────────────────────

export const IconChat = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
        <line x1="8" y1="7" x2="16" y2="7" />
        <line x1="8" y1="10" x2="16" y2="10" />
        <line x1="8" y1="13" x2="12" y2="13" />
    </svg>
);

export const IconFileText = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <polyline points="14 2 14 8 20 8" />
        <line x1="8" y1="10" x2="16" y2="10" />
        <line x1="8" y1="14" x2="16" y2="14" />
        <line x1="8" y1="18" x2="16" y2="18" />
    </svg>
);

export const IconSparkle = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polygon points="11 4 13 11 20 13 13 15 11 22 9 15 2 13 9 11" />
        <polygon points="18 3 19 5 21 6 19 7 18 9 17 7 15 6 17 5" />
    </svg>
);

export const IconCode = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polyline points="16 18 22 12 16 6" />
        <polyline points="8 6 2 12 8 18" />
        <line x1="14" y1="4" x2="10" y2="20" />
    </svg>
);

export const IconTie = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <polygon points="9 4 15 4 13 8 11 8" />
        <polygon points="11 8 13 8 16 18 12 22 8 18" />
    </svg>
);

// ── Empty States & Info ───────────────────────────────────────────────

export const IconBook = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M2 3h6a4 4 0 0 1 4 4v14a3 3 0 0 0-3-3H2z" />
        <path d="M22 3h-6a4 4 0 0 0-4 4v14a3 3 0 0 1 3-3h7z" />
        <line x1="6" y1="8" x2="9" y2="8" />
        <line x1="6" y1="12" x2="9" y2="12" />
        <line x1="15" y1="8" x2="18" y2="8" />
        <line x1="15" y1="12" x2="18" y2="12" />
    </svg>
);

export const IconFileLightning = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
        <polyline points="14 2 14 8 20 8" />
        <polygon points="13 11 9 17 12 17 11 21 15 15 12 15 13 11" />
    </svg>
);

export const IconLightbulb = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M15 14c.2-1 .7-1.7 1.5-2.5 1-.9 1.5-2.2 1.5-3.5A6 6 0 0 0 6 8c0 1 .2 2.2 1.5 3.5.7.9 1.2 1.5 1.5 2.5" />
        <path d="M9 14h6v4H9z" />
        <line x1="9" y1="16" x2="15" y2="16" />
        <path d="M10 18v1.5a2 2 0 0 0 4 0V18" />
        <path d="M10 14v-2.5a2 2 0 0 1 4 0V14" />
    </svg>
);

// ── Utility ───────────────────────────────────────────────────────────

export const IconSettings = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <circle cx="12" cy="12" r="3" />
        <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
    </svg>
);

export const IconCopy = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <path d="M7 18H6a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h8a2 2 0 0 1 2 2v3" />
        <path d="M15 8h2a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H9a2 2 0 0 1-2-2V10a2 2 0 0 1 2-2h2" />
        <rect x="11" y="6" width="4" height="4" rx="1" ry="1" />
    </svg>
);

export const IconKeyboard = ({ size = 16, ...props }: IconProps) => (
    <svg {...defaultProps(size)} stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" {...props}>
        <rect x="2" y="5" width="20" height="14" rx="2" ry="2" />
        <line x1="6" y1="9" x2="6.01" y2="9" /><line x1="10" y1="9" x2="10.01" y2="9" />
        <line x1="14" y1="9" x2="14.01" y2="9" /><line x1="18" y1="9" x2="18.01" y2="9" />
        <line x1="6" y1="12" x2="6.01" y2="12" /><line x1="10" y1="12" x2="10.01" y2="12" />
        <line x1="14" y1="12" x2="14.01" y2="12" /><line x1="18" y1="12" x2="18.01" y2="12" />
        <line x1="8" y1="15" x2="16" y2="15" />
    </svg>
);
