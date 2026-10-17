import React from "react";
import type { RegisteredApp } from "./types";
import { MultiViewWorkspace } from "./MultiViewWorkspace";

interface PublishedAppViewProps {
  app: RegisteredApp;
  onOpenBuilder?: (slug: string) => void;
  onOpenIntakeForm?: (tableId?: string) => void;
  onBackToDesk?: () => void;
}

/**
 * The published app screen: the app's own tables, and the rows the server holds for it. It builds its columns
 * from the app's definition, so it works for an app of any shape. It holds no rows of its own.
 */
export const PublishedAppView: React.FC<PublishedAppViewProps> = ({ app, onOpenBuilder, onOpenIntakeForm, onBackToDesk }) => (
  <div data-testid="published-app-runtime" className="min-h-screen bg-slate-50 dark:bg-slate-950">
    {onOpenIntakeForm && (
      <div className="px-6 pt-3 flex justify-end">
        <button
          type="button"
          data-testid="open-public-intake-form-btn"
          onClick={() => onOpenIntakeForm()}
          className="px-3 py-1.5 rounded-lg border border-blue-200 bg-blue-50 text-blue-700 text-xs font-semibold hover:bg-blue-100 cursor-pointer"
        >
          📋 Public Intake Form
        </button>
      </div>
    )}
    <MultiViewWorkspace
      app={app}
      onBack={() => onBackToDesk?.()}
      onOpenStudio={() => onOpenBuilder?.(app.slug)}
    />
  </div>
);
