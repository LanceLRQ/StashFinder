const paths = {
  home: "m2 9 8-7 8 7 M4 8v10h4v-5h4v5h4V8",
  folder: "M2 5h5l2 2h9v10H2z",
  drive: "M3 12h14v5H3z M3 12l3-7h8l3 7 M6 14.5h1",
  pencil: "M4 13 13 4l3 3-9 9-4 1 1-4z M11 6l3 3",
  search: "M14 9a5 5 0 1 1-10 0 5 5 0 0 1 10 0Zm-1 4 5 5",
  filter: "M3 5h14l-5.5 6v4l-3 2v-6z",
  check: "m4 10 4 4 8-8",
  tasks: "M4 4h12v14H4z M7 8h6 M7 12h6",
  settings: "M3 6h14 M3 14h14 M7 3v6 M13 11v6",
  development: "m7 5-5 5 5 5 m6-10 5 5-5 5",
  chevronRight: "m8 5 5 5-5 5",
  chevronDown: "m5 8 5 5 5-5",
  close: "m5 5 10 10 M15 5 5 15",
};

export default function WorkspaceIcon({
  name,
  className,
}: {
  name: keyof typeof paths;
  className?: string;
}) {
  return (
    <svg
      className={className}
      width="16"
      height="16"
      viewBox="0 0 20 20"
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d={paths[name]} />
    </svg>
  );
}
