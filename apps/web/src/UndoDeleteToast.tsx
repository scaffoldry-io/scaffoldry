import { useState } from "react";
import { restoreTrash } from "./api";
import { Toast } from "./ui";

/** Shown after a record is deleted. `Undo` brings the whole delete batch back. */
export function UndoDeleteToast({
  appSlug,
  deleteBatch,
  onClose,
  onRestored,
}: {
  appSlug: string;
  deleteBatch: string;
  onClose: () => void;
  onRestored?: () => void;
}) {
  const [failure, setFailure] = useState<unknown>(null);
  if (failure !== null) return <Toast tone="error" error={failure} onClose={onClose} />;
  const undo = async () => {
    try {
      await restoreTrash(appSlug, deleteBatch);
      onRestored?.();
      onClose();
    } catch (e) {
      setFailure(e);
    }
  };
  return (
    <>
      <Toast tone="success" message="Moved to trash." onClose={onClose} autoDismissMs={8000} />
      <button type="button" onClick={undo} className="fixed bottom-5 right-24 z-50 underline text-sm">
        Undo
      </button>
    </>
  );
}
