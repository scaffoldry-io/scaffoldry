import React from "react";
import { Drawer } from "./ui";

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
  return (
    <Drawer open={isOpen} onClose={onClose} title="Create New Application Table" variant="modal" size="md">
      <div className="space-y-4">
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
    </Drawer>
  );
};
