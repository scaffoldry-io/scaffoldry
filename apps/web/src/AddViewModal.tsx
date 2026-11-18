import React from "react";
import { ViewType } from "./types";
import { Drawer } from "./ui";

export interface AddViewModalProps {
  isOpen: boolean;
  onClose: () => void;
  newViewTitle: string;
  setNewViewTitle: (title: string) => void;
  newViewType: ViewType;
  setNewViewType: (vt: ViewType) => void;
  onCreateNewView: (e: React.FormEvent) => void;
}

export const AddViewModal: React.FC<AddViewModalProps> = ({
  isOpen,
  onClose,
  newViewTitle,
  setNewViewTitle,
  newViewType,
  setNewViewType,
  onCreateNewView,
}) => {
  return (
    <Drawer open={isOpen} onClose={onClose} title="Create New Saved View" variant="modal" size="md" testId="modal-add-view">
      <div className="space-y-4">
        <form onSubmit={onCreateNewView} className="space-y-4 text-xs">
          <div>
            <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
              View Title
            </label>
            <input
              type="text"
              required
              placeholder="e.g. Approved Grants Kanban"
              value={newViewTitle}
              onChange={(e) => setNewViewTitle(e.target.value)}
              className="w-full px-3 py-2 rounded-lg border border-slate-300 dark:border-slate-700 bg-slate-50 dark:bg-slate-800 text-slate-900 dark:text-white"
            />
          </div>

          <div>
            <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
              View Type
            </label>
            <div className="grid grid-cols-2 gap-2">
              {(["Grid", "Kanban", "Calendar", "Gallery"] as ViewType[]).map((vt) => (
                <button
                  key={vt}
                  type="button"
                  onClick={() => setNewViewType(vt)}
                  className={`p-2.5 rounded-lg border text-left cursor-pointer transition-all ${
                    newViewType === vt
                      ? "border-blue-600 bg-blue-50/60 dark:bg-blue-950/60 text-blue-700 dark:text-blue-300"
                      : "border-slate-200 dark:border-slate-700 hover:bg-slate-50 dark:hover:bg-slate-800 text-slate-700 dark:text-slate-300"
                  }`}
                >
                  <div className="font-bold flex items-center gap-1.5">
                    <span>
                      {vt === "Grid"
                        ? "▦"
                        : vt === "Kanban"
                        ? "☷"
                        : vt === "Calendar"
                        ? "📅"
                        : "🖼"}
                    </span>
                    <span>{vt}</span>
                  </div>
                  <div className="text-[10px] text-slate-500 mt-0.5">
                    {vt === "Grid"
                      ? "Tabular records matrix"
                      : vt === "Kanban"
                      ? "Card stage progression"
                      : vt === "Calendar"
                      ? "Timeline agenda"
                      : "Visual card gallery"}
                  </div>
                </button>
              ))}
            </div>
          </div>

          <div className="flex items-center justify-end gap-2 pt-3 border-t border-slate-100 dark:border-slate-800">
            <button
              type="button"
              onClick={onClose}
              className="px-3.5 py-1.5 rounded-lg border border-slate-300 dark:border-slate-700 text-slate-600 dark:text-slate-400 font-semibold cursor-pointer"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="px-4 py-1.5 rounded-lg bg-blue-600 hover:bg-blue-700 text-white font-bold shadow-xs cursor-pointer"
            >
              Create View
            </button>
          </div>
        </form>
      </div>
    </Drawer>
  );
};
