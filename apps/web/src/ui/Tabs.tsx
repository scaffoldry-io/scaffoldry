import { useRef, type KeyboardEvent } from "react";

export interface TabItem {
  id: string;
  label: string;
}

export interface TabsProps {
  tabs: TabItem[];
  selectedId: string;
  onSelect: (id: string) => void;
  ariaLabel?: string;
}

export function Tabs({ tabs, selectedId, onSelect, ariaLabel }: TabsProps) {
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const handleKeyDown = (e: KeyboardEvent<HTMLButtonElement>, index: number) => {
    let nextIndex = index;
    if (e.key === "ArrowRight") {
      nextIndex = (index + 1) % tabs.length;
    } else if (e.key === "ArrowLeft") {
      nextIndex = (index - 1 + tabs.length) % tabs.length;
    } else if (e.key === "Home") {
      nextIndex = 0;
    } else if (e.key === "End") {
      nextIndex = tabs.length - 1;
    } else {
      return;
    }

    e.preventDefault();
    onSelect(tabs[nextIndex].id);
    tabRefs.current[nextIndex]?.focus();
  };

  return (
    <div
      role="tablist"
      aria-label={ariaLabel}
      className="flex space-x-1 border-b border-gray-200 dark:border-gray-800"
    >
      {tabs.map((tab, idx) => {
        const isSelected = tab.id === selectedId;
        return (
          <button
            key={tab.id}
            ref={(el) => {
              tabRefs.current[idx] = el;
            }}
            role="tab"
            aria-selected={isSelected}
            tabIndex={isSelected ? 0 : -1}
            onClick={() => onSelect(tab.id)}
            onKeyDown={(e) => handleKeyDown(e, idx)}
            className={`px-4 py-2 text-sm font-medium border-b-2 -mb-px transition-colors ${
              isSelected
                ? "border-blue-600 text-blue-600 dark:border-blue-400 dark:text-blue-400"
                : "border-transparent text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
            }`}
          >
            {tab.label}
          </button>
        );
      })}
    </div>
  );
}
