export interface CalculatingNoteProps {
  /** How far along the job is, from 0 to 99. Omit when it is not known. */
  percent?: number;
}

/** What a column header shows while the server writes a field's values again: `Calculating, 63% done`. */
export function CalculatingNote({ percent }: CalculatingNoteProps) {
  const words = percent === undefined ? "Calculating" : `Calculating, ${Math.round(percent)}% done`;
  return (
    <span role="status" className="inline-flex items-center gap-1 text-xs text-gray-600 dark:text-gray-300">
      <span aria-hidden="true" className="inline-block h-3 w-3 animate-spin rounded-full border-2 border-current border-t-transparent" />
      {words}
    </span>
  );
}
