import "@testing-library/jest-dom";
import { describe, it, expect, beforeEach } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import React from "react";
import { parseDuration, formatDuration, formatDateTime, parseDateTimeInput } from "../utils/durationDatetime";
import { DurationEditor } from "../DurationEditor";
import { UserSettingsModal } from "../UserSettingsModal";

describe("Duration and DateTime Phase 4", () => {
  beforeEach(() => {
    localStorage.clear();
  });

  describe("Duration Parsing & Formatting", () => {
    it("parses 90m, 1.5h, and 1:30 into 5400 seconds", () => {
      expect(parseDuration("90m")).toBe(5400);
      expect(parseDuration("1.5h")).toBe(5400);
      expect(parseDuration("1:30")).toBe(5400);
      expect(parseDuration("5400")).toBe(5400);
    });

    it("parses day formats such as 2d and 1d 2h", () => {
      expect(parseDuration("2d")).toBe(172800);
      expect(parseDuration("1d 2h")).toBe(86400 + 7200);
    });

    it("formats durations according to specified format", () => {
      // 5400 seconds = 1 hr 30 min
      expect(formatDuration(5400, "h:mm")).toBe("1:30");
      expect(formatDuration(5400, "h:mm:ss")).toBe("1:30:00");
      expect(formatDuration(5400, "d h")).toBe("0d 1h");
      expect(formatDuration(93600, "d h")).toBe("1d 2h");
      expect(formatDuration(172800, "days")).toBe("2 days");
      expect(formatDuration(86400, "days")).toBe("1 day");
      expect(formatDuration(129600, "days")).toBe("1.5 days");
    });

    it("rejects invalid duration input with null or error", () => {
      expect(parseDuration("invalid")).toBeNull();
      expect(parseDuration("abc")).toBeNull();
      expect(parseDuration("-5m")).toBeNull();
    });
  });

  describe("DurationEditor component", () => {
    it("shows live interpretation preview before commit and commits valid duration", () => {
      let committedVal: number | null = null;
      const onChange = (val: number | null) => {
        committedVal = val;
      };

      render(
        <DurationEditor
          id="duration-test"
          label="Estimated Time"
          value={null}
          format="h:mm"
          onChange={onChange}
        />
      );

      const input = screen.getByLabelText("Estimated Time");
      
      // Type "90m"
      fireEvent.change(input, { target: { value: "90m" } });
      
      // Expect live interpretation to be shown
      expect(screen.getByText(/1h 30m|5,400s|1 hr 30 min/i)).toBeInTheDocument();

      // Blur or press Enter to commit
      fireEvent.blur(input);
      expect(committedVal).toBe(5400);
    });

    it("shows error message for invalid input and does not commit", () => {
      let committedVal: any = "untouched";
      const onChange = (val: any) => {
        committedVal = val;
      };

      render(
        <DurationEditor
          id="duration-bad"
          label="Run Time"
          value={null}
          format="h:mm"
          onChange={onChange}
        />
      );

      const input = screen.getByLabelText("Run Time");
      fireEvent.change(input, { target: { value: "garbage-input" } });
      
      // Expect error message
      expect(screen.getByText(/invalid duration/i)).toBeInTheDocument();

      fireEvent.blur(input);
      expect(committedVal).toBe("untouched");
    });
  });

  describe("User Settings Timezone and DateTime handling", () => {
    it("allows selecting user timezone in UserSettingsModal and persists to localStorage", () => {
      render(
        <UserSettingsModal
          isOpen={true}
          onClose={() => {}}
          eppn="prof.curie@science.state.edu"
          agentEnabled={false}
          maxDays={30}
        />
      );

      const tzSelect = screen.getByLabelText(/time zone/i);
      expect(tzSelect).toBeInTheDocument();

      fireEvent.change(tzSelect, { target: { value: "America/New_York" } });
      expect(localStorage.getItem("scaffoldry_user_timezone")).toBe("America/New_York");
    });

    it("formats DateTime in given timezone", () => {
      const instant = "2026-10-12T03:30:00Z";
      // In America/New_York (UTC-4 in Oct): 2026-10-11 23:30
      const ny = formatDateTime(instant, "America/New_York");
      expect(ny).toContain("2026-10-11 23:30");

      // In UTC: 2026-10-12 03:30
      const utc = formatDateTime(instant, "UTC");
      expect(utc).toContain("2026-10-12 03:30");
    });

    it("parses local DateTime input in user timezone into UTC instant", () => {
      // User enters "2026-10-11T23:30" while in America/New_York
      const utcInstant = parseDateTimeInput("2026-10-11T23:30", "America/New_York");
      expect(utcInstant).toBe("2026-10-12T03:30:00Z");
    });
  });
});
