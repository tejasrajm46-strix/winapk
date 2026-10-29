/**
 * A small inline icon set. Stroke geometry follows the reference's thin,
 * precise line work; there is no icon font and no remote asset, so the app
 * renders identically offline.
 */

import type { SVGProps } from "react";

type IconProps = SVGProps<SVGSVGElement> & { size?: number };

function base({ size = 16, ...props }: IconProps) {
  return {
    width: size,
    height: size,
    viewBox: "0 0 24 24",
    fill: "none",
    stroke: "currentColor",
    strokeWidth: 1.5,
    strokeLinecap: "square" as const,
    strokeLinejoin: "miter" as const,
    "aria-hidden": true,
    focusable: false,
    ...props,
  };
}

export const IconOverview = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="3" y="3" width="7" height="7" />
    <rect x="14" y="3" width="7" height="7" />
    <rect x="3" y="14" width="7" height="7" />
    <rect x="14" y="14" width="7" height="7" />
  </svg>
);

export const IconApplications = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="3" y="4" width="8" height="8" />
    <rect x="13" y="4" width="8" height="8" />
    <rect x="3" y="14" width="8" height="6" />
    <rect x="13" y="14" width="8" height="6" />
  </svg>
);

export const IconPhone = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="7" y="2" width="10" height="20" />
    <path d="M10 5h4M11 19h2" />
  </svg>
);

export const IconInstall = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M12 3v11" />
    <path d="M8 10.5 12 14l4-3.5" />
    <path d="M4 18v3h16v-3" />
  </svg>
);

export const IconRuntime = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="6" y="6" width="12" height="12" />
    <path d="M9 3v3M15 3v3M9 18v3M15 18v3M3 9h3M3 15h3M18 9h3M18 15h3" />
  </svg>
);

export const IconActivity = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M3 7l4 4-4 4" />
    <path d="M11 19h10" />
    <path d="M11 12h6" />
    <path d="M11 5h10" />
  </svg>
);

export const IconAgent = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="4" y="7" width="16" height="12" />
    <path d="M12 3v4" />
    <path d="M9 12v2M15 12v2" />
    <path d="M2 12v4M22 12v4" />
  </svg>
);

export const IconSettings = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="3" />
    <path d="M12 2v3M12 19v3M2 12h3M19 12h3M5 5l2 2M17 17l2 2M19 5l-2 2M7 17l-2 2" />
  </svg>
);

export const IconSearch = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="11" cy="11" r="6" />
    <path d="M16 16l4 4" />
  </svg>
);

export const IconGrid = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="3" y="3" width="7" height="7" />
    <rect x="14" y="3" width="7" height="7" />
    <rect x="3" y="14" width="7" height="7" />
    <rect x="14" y="14" width="7" height="7" />
  </svg>
);

export const IconList = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 6h2M9 6h11M4 12h2M9 12h11M4 18h2M9 18h11" />
  </svg>
);

export const IconCheck = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 12.5 9 17.5 20 6.5" />
  </svg>
);

export const IconAlert = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M12 3 22 21H2L12 3z" />
    <path d="M12 9v6" />
    <path d="M12 18h.01" />
  </svg>
);

export const IconClose = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M5 5l14 14M19 5 5 19" />
  </svg>
);

export const IconPlay = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M7 4l12 8-12 8V4z" />
  </svg>
);

export const IconStop = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="6" y="6" width="12" height="12" />
  </svg>
);

export const IconTrash = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 7h16" />
    <path d="M9 7V4h6v3" />
    <path d="M6 7l1 14h10l1-14" />
    <path d="M10 11v6M14 11v6" />
  </svg>
);

export const IconRefresh = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M20 12a8 8 0 1 1-2.5-5.8" />
    <path d="M20 3v5h-5" />
  </svg>
);

export const IconPackage = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M12 3l9 5v8l-9 5-9-5V8l9-5z" />
    <path d="M3 8l9 5 9-5" />
    <path d="M12 13v8" />
  </svg>
);

export const IconFolder = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M3 6h6l2 2h10v11H3V6z" />
  </svg>
);

export const IconClock = (p: IconProps) => (
  <svg {...base(p)}>
    <circle cx="12" cy="12" r="9" />
    <path d="M12 7v5l4 2" />
  </svg>
);

/** Android navigation: a hardware-style back arrow. */
export const IconBack = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M19 12H5" />
    <path d="M10 6l-5 6 5 6" />
  </svg>
);

export const IconHome = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M4 10.5 12 4l8 6.5" />
    <path d="M6 10v10h12V10" />
  </svg>
);

/** Recent apps: two stacked cards, the Android overview shape. */
export const IconRecents = (p: IconProps) => (
  <svg {...base(p)}>
    <rect x="4" y="4" width="16" height="10" />
    <path d="M4 17h16" />
  </svg>
);

export const IconCamera = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M3 8h4l2-3h6l2 3h4v11H3z" />
    <circle cx="12" cy="13" r="3.2" />
  </svg>
);

/** A filled dot: the record control, and the indicator while it runs. */
export const IconRecord = ({ size = 16, ...props }: IconProps) => (
  <svg {...base({ size, ...props })} fill="currentColor" stroke="none">
    <circle cx="12" cy="12" r="6" />
  </svg>
);

export const IconOpen = (p: IconProps) => (
  <svg {...base(p)}>
    <path d="M14 4h6v6" />
    <path d="M20 4l-8 8" />
    <path d="M18 14v6H4V6h6" />
  </svg>
);
