import React from "react";

export interface CreateTableModalProps {
  isOpen: boolean;
  onClose: () => void;
  newTableName: string;
  setNewTableName: (name: string) => void;
  newTableIcon: string;
  setNewTableIcon: (icon: string) => void;
  onAddNewTable: (e: React.FormEvent) => void;
}

export const CreateTableModal: React.FC<CreateTableModalProps> = ({
  isOpen,
  onClose,
  newTableName,
  setNewTableName,
  newTableIcon,
  setNewTableIcon,
  onAddNewTable,
}) => {
  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-xs animate-fade-in">
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-2xl w-full max-w-md p-6 space-y-4">
        <div className="flex items-center justify-between pb-3 border-b border-slate-200 dark:border-slate-800">
          <h3 className="text-sm font-bold text-slate-900 dark:text-white">Create New Application Table</h3>
          <button
            type="button"
            onClick={onClose}
            className="text-slate-400 hover:text-slate-600 text-sm"
          >
            ✕
          </button>
        </div>
        <form onSubmit={onAddNewTable} className="space-y-3 text-xs">
          <div>
            <label className="font-semibold text-slate-700 dark:text-slate-300">Table Name</label>
            <input
              type="text"
              required
              placeholder="e.g. Milestone Reviews"
              value={newTableName}
              onChange={(e) => setNewTableName(e.target.value)}
              className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
            />
          </div>
          <div>
            <label className="font-semibold text-slate-700 dark:text-slate-300">Table Icon</label>
            <input
              type="text"
              value={newTableIcon}
              onChange={(e) => setNewTableIcon(e.target.value)}
              className="mt-1 w-full px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-800"
            />
          </div>
          <div className="flex items-center justify-end gap-2 pt-2">
            <button
              type="button"
              onClick={onClose}
              className="px-3 py-1.5 rounded border border-slate-300 dark:border-slate-700"
            >
              Cancel
            </button>
            <button
              type="submit"
              className="px-3.5 py-1.5 rounded bg-blue-600 text-white font-bold"
            >
              Create Table
            </button>
          </div>
        </form>
      </div>
    </div>
  );
};
