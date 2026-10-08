import type { CSSProperties } from "react";
export type IconName =
  | "home"
  | "memory"
  | "sources"
  | "link"
  | "activity"
  | "settings"
  | "search"
  | "plus"
  | "arrow"
  | "close"
  | "chevron"
  | "check"
  | "alert"
  | "folder"
  | "shield"
  | "file"
  | "download"
  | "edit"
  | "external"
  | "pause"
  | "play"
  | "refresh"
  | "dots"
  | "back"
  | "moon"
  | "code"
  | "book"
  | "globe";
const paths: Record<IconName, string> = {
  home: "M3 10 12 3l9 7 M5 9v12h5v-7h4v7h5V9",
  memory: "M6 3h9l3 3v15H6z M14 3v5h4 M9 12h6 M9 16h6",
  sources: "M3 6h6l2 2h10v12H3z M3 6V4h6l2 2h8v2",
  link: "M10 13a5 5 0 0 0 7 0l3-3a5 5 0 0 0-7-7l-2 2 M14 11a5 5 0 0 0-7 0l-3 3a5 5 0 0 0 7 7l2-2",
  activity: "M3 12h4l3-8 4 16 3-8h4",
  settings:
    "M9 3h6l1 4 4 1v6l-4 1-1 4H9l-1-4-4-1V8l4-1z M12 9a3 3 0 1 0 0 6 3 3 0 0 0 0-6",
  search: "M16 16l5 5 M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14",
  plus: "M12 5v14 M5 12h14",
  arrow: "M5 12h14 M14 7l5 5-5 5",
  close: "M6 6l12 12 M6 18 18 6",
  chevron: "M9 5l7 7-7 7",
  check: "M5 12l4 4L19 6",
  alert: "M12 3 2 21h20z M12 9v5 M12 17v1",
  folder: "M3 5h6l2 2h10v13H3z",
  shield: "M12 3 4 6v6c0 5 8 9 8 9s8-4 8-9V6z M8 12l3 3 5-6",
  file: "M5 3h9l5 5v13H5z M14 3v6h5",
  download: "M12 3v12 M7 10l5 5 5-5 M4 16v5h16v-5",
  edit: "M14 4l6 6 M3 21l5-1L21 7l-5-5L3 15z",
  external: "M14 3h7v7 M21 3 11 13 M10 4H4v16h16v-6",
  pause: "M8 5v14 M16 5v14",
  play: "M7 4l13 8-13 8z",
  refresh: "M20 8A8 8 0 1 0 20 16 M20 2v6h-6",
  dots: "M5 12h.1 M12 12h.1 M19 12h.1",
  back: "M19 12H5 M10 7l-5 5 5 5",
  moon: "M20 15A9 9 0 0 1 9 4a9 9 0 1 0 11 11",
  code: "M8 6l-6 6 6 6 M16 6l6 6-6 6 M14 3l-4 18",
  book: "M12 6C8 3 3 4 3 4v15s5-1 9 2c4-3 9-2 9-2V4s-5-1-9 2z M12 6v15",
  globe:
    "M3 12h18 M12 3c6 6 6 12 0 18-6-6-6-12 0-18 M12 3a9 9 0 1 0 0 18 9 9 0 0 0 0-18",
};
export function Icon({
  name,
  size = 18,
  className = "",
  style,
}: {
  name: IconName;
  size?: number;
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <svg
      aria-hidden="true"
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.65"
      strokeLinecap="round"
      strokeLinejoin="round"
      className={`icon ${className}`}
      style={style}
    >
      <path d={paths[name]} />
    </svg>
  );
}
