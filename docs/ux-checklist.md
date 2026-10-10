# UX and Accessibility Review Checklist

A reviewer verifies and ticks this checklist for every new screen and modified view before merging.

---

### 1. The Four Asynchronous States
- [ ] **Working:** Clear visual loading indicator (e.g., `Skeleton` rows or spinner). Respects `prefers-reduced-motion`. Action buttons disabled during execution to prevent duplicate requests.
- [ ] **Done:** Complete data rendered cleanly. Success feedback uses polite announcements (`Toast` with `role="status"`).
- [ ] **Empty:** Informative `EmptyState` when collections or searches return no rows. Explains why and provides at most one direct recovery action.
- [ ] **Failed:** Visible `ErrorState` with a clear explanation, guidance on what to do next, a `Retry` action where feasible, and a disclosure for technical details (status code, error code).

### 2. Keyboard Navigation & Focus Flow
- [ ] **Logical Tab Order:** Focus traverses the screen according to reading order. Focus ring is visibly distinct in both light and dark modes.
- [ ] **Drawer / Dialog Trapping:** Opening a `Drawer` moves focus inside. Tab cycles within the container without escaping.
- [ ] **Escape to Close:** Pressing Escape immediately closes the open drawer or modal.
- [ ] **Focus Return:** Closing a drawer or dialog returns focus to the exact element that opened it.
- [ ] **Interactive Tables:** Rows are keyboard-focusable (`tabIndex={0}`) and trigger selection on pressing Enter.
- [ ] **Tabs Control:** Tab strip implements WAI-ARIA `tablist` pattern with arrow-key switching (`ArrowLeft`, `ArrowRight`, `Home`, `End`).

### 3. Decisions, Reasons & Consequences
- [ ] **Action Framing:** Button labels use concrete verb + noun pairs (e.g., `Hold Alex Smith`, not generic `Submit`).
- [ ] **Clear Consequence:** An explicit sentence states the outcome before an irreversible action is executed.
- [ ] **In-Place Justification:** Any action requiring an administrative justification presents an in-place reason field next to the action button, never a prompt.
- [ ] **Audited Actions:** Operations recorded in the SHA-256 tamper-evident ledger clearly state: *This is recorded in the audit ledger.*

### 4. Identification & Terminology
- [ ] **Names, Not Raw IDs:** People display with name and address (`PersonLabel`), organizational units display with title and code (`UnitLabel`), and controls display with full NIST title and identifier (`ControlLabel`). Bare UUIDs or foreign keys are never standalone labels.
- [ ] **Plain English:** Clear institutional wording. Jargon and protocol terms (e.g., `eppn`, `Cedar`, `ABAC`) carry descriptive labels on first appearance.
- [ ] **Multi-Signal Status:** Color is never the sole communicator of status. Every `StatusBadge` couples tone with clear text and a distinct symbol.

### 5. Data Authenticity & Browser Standards
- [ ] **No Sample Data:** No hardcoded mocks masquerade as live enterprise state. Unavailable endpoints fail visibly.
- [ ] **No Native Browser Dialogs:** No `window.alert()`, `window.confirm()`, or `window.prompt()`. All interactions employ kit components (`ConfirmAction`, `Drawer`, `Toast`).
- [ ] **Dark Mode Harmony:** Elements utilize Tailwind dark theme tokens (`dark:*`). Contrast passes WCAG 2.2 Level AA requirements across all tones and backgrounds.
