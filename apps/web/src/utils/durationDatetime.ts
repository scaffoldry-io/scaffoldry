/**
 * Duration and DateTime utility functions for Scaffoldry.
 */

export const DURATION_FORMATS = ["h:mm", "h:mm:ss", "d h", "days"] as const;
export type DurationFormat = typeof DURATION_FORMATS[number];

export const COMMON_TIMEZONES = [
  "UTC",
  "America/New_York",
  "America/Chicago",
  "America/Denver",
  "America/Los_Angeles",
  "America/Anchorage",
  "Pacific/Honolulu",
  "Europe/London",
  "Europe/Paris",
  "Europe/Berlin",
  "Asia/Dubai",
  "Asia/Tokyo",
  "Asia/Shanghai",
  "Asia/Singapore",
  "Australia/Sydney",
];

/**
 * Parses duration input string into whole seconds.
 * Accepts: "90m", "1.5h", "1:30", "2d", "1d 2h", "30s", "5400".
 * Returns null if invalid or negative.
 */
export function parseDuration(input: string | number | null | undefined): number | null {
  if (input === null || input === undefined) return null;
  if (typeof input === "number") {
    return input >= 0 && !isNaN(input) ? Math.round(input) : null;
  }

  const s = String(input).trim();
  if (!s || s.startsWith("-")) return null;

  // Pure integer
  if (/^\d+$/.test(s)) {
    return parseInt(s, 10);
  }

  // Colon notation: h:mm or h:mm:ss
  if (s.includes(":")) {
    const parts = s.split(":");
    if (parts.length === 2) {
      const h = parseInt(parts[0].trim(), 10);
      const m = parseInt(parts[1].trim(), 10);
      if (!isNaN(h) && !isNaN(m) && h >= 0 && m >= 0 && m <= 59) {
        return h * 3600 + m * 60;
      }
    } else if (parts.length === 3) {
      const h = parseInt(parts[0].trim(), 10);
      const m = parseInt(parts[1].trim(), 10);
      const sec = parseInt(parts[2].trim(), 10);
      if (!isNaN(h) && !isNaN(m) && !isNaN(sec) && h >= 0 && m >= 0 && m <= 59 && sec >= 0 && sec <= 59) {
        return h * 3600 + m * 60 + sec;
      }
    }
    return null;
  }

  // Check for junk characters
  const cleaned = s.replace(/(\d+(?:\.\d+)?)\s*([a-zA-Z]+)/g, "").trim();
  if (cleaned.length > 0) {
    return null;
  }

  // Unit notation e.g. "90m", "1.5h", "2d", "1d 2h"
  const regex = /(\d+(?:\.\d+)?)\s*([a-zA-Z]+)/g;
  let match: RegExpExecArray | null;
  let totalSecs = 0;
  let matchedAny = false;

  while ((match = regex.exec(s)) !== null) {
    matchedAny = true;
    const num = parseFloat(match[1]);
    const unit = match[2].toLowerCase();

    let mult = 0;
    if (["s", "sec", "secs", "second", "seconds"].includes(unit)) {
      mult = 1;
    } else if (["m", "min", "mins", "minute", "minutes"].includes(unit)) {
      mult = 60;
    } else if (["h", "hr", "hrs", "hour", "hours"].includes(unit)) {
      mult = 3600;
    } else if (["d", "day", "days"].includes(unit)) {
      mult = 86400;
    } else {
      return null;
    }

    totalSecs += num * mult;
  }

  return matchedAny && totalSecs >= 0 ? Math.round(totalSecs) : null;
}

/**
 * Formats seconds according to the specified format.
 */
export function formatDuration(seconds: number | null | undefined, format: string = "h:mm"): string {
  if (seconds === null || seconds === undefined || isNaN(seconds)) return "—";
  const s = Math.max(0, Math.round(seconds));

  switch (format) {
    case "h:mm:ss": {
      const h = Math.floor(s / 3600);
      const m = Math.floor((s % 3600) / 60);
      const sec = s % 60;
      return `${h}:${m.toString().padStart(2, "0")}:${sec.toString().padStart(2, "0")}`;
    }
    case "d h": {
      const d = Math.floor(s / 86400);
      const h = Math.floor((s % 86400) / 3600);
      return `${d}d ${h}h`;
    }
    case "days": {
      if (s % 86400 === 0) {
        const d = s / 86400;
        return `${d} ${d === 1 ? "day" : "days"}`;
      }
      const d = +(s / 86400).toFixed(2);
      return `${d} days`;
    }
    case "h:mm":
    default: {
      const h = Math.floor(s / 3600);
      const m = Math.floor((s % 3600) / 60);
      return `${h}:${m.toString().padStart(2, "0")}`;
    }
  }
}

/**
 * Returns human-readable interpretation of duration in words.
 */
export function describeDuration(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  const days = Math.floor(s / 86400);
  const hours = Math.floor((s % 86400) / 3600);
  const mins = Math.floor((s % 3600) / 60);
  const secs = s % 60;

  const parts: string[] = [];
  if (days > 0) parts.push(`${days} day${days > 1 ? "s" : ""}`);
  if (hours > 0) parts.push(`${hours} hr${hours > 1 ? "s" : ""}`);
  if (mins > 0) parts.push(`${mins} min${mins > 1 ? "s" : ""}`);
  if (secs > 0 || parts.length === 0) parts.push(`${secs} sec${secs !== 1 ? "s" : ""}`);

  return `${parts.join(" ")} (${s.toLocaleString()}s)`;
}

export function getUserTimezone(): string {
  try {
    return localStorage.getItem("scaffoldry_user_timezone") || "UTC";
  } catch {
    return "UTC";
  }
}

export function setUserTimezone(tz: string): void {
  try {
    localStorage.setItem("scaffoldry_user_timezone", tz);
  } catch {
    // Ignore storage errors
  }
}

/**
 * Formats a UTC ISO-8601 string into localized date and time in the given timezone.
 */
export function formatDateTime(utcIso: string | null | undefined, timeZone?: string): string {
  if (!utcIso) return "—";
  const tz = timeZone || getUserTimezone();
  const date = new Date(utcIso);
  if (isNaN(date.getTime())) return String(utcIso);

  const formatter = new Intl.DateTimeFormat("en-US", {
    timeZone: tz,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });

  const parts = formatter.formatToParts(date);
  const map: Record<string, string> = {};
  for (const p of parts) {
    if (p.type !== "literal") {
      map[p.type] = p.value;
    }
  }

  // Format as YYYY-MM-DD HH:mm:ss
  const hour = map.hour === "24" ? "00" : map.hour;
  return `${map.year}-${map.month}-${map.day} ${hour}:${map.minute}:${map.second}`;
}

/**
 * Parses local datetime input string (e.g. "2026-10-11T23:30") in a given timeZone
 * into a UTC ISO-8601 instant string (e.g. "2026-10-12T03:30:00Z").
 */
export function parseDateTimeInput(localStr: string, timeZone: string = "UTC"): string | null {
  if (!localStr) return null;
  const match = localStr.match(/^(\d{4})-(\d{2})-(\d{2})(?:[T ](\d{2}):(\d{2})(?::(\d{2}))?)?/);
  if (!match) return null;
  const [_, y, mo, d, h = "00", mi = "00", s = "00"] = match;

  const guessIso = `${y}-${mo}-${d}T${h}:${mi}:${s}Z`;
  const guessDate = new Date(guessIso);
  if (isNaN(guessDate.getTime())) return null;

  if (timeZone === "UTC") {
    return guessDate.toISOString().replace(/\.\d{3}Z$/, "Z");
  }

  const formatter = new Intl.DateTimeFormat("en-US", {
    timeZone,
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });

  const getParts = (dt: Date) => {
    const parts = formatter.formatToParts(dt);
    const map: Record<string, number> = {};
    for (const p of parts) {
      if (p.type !== "literal") {
        map[p.type] = parseInt(p.value, 10);
      }
    }
    return Date.UTC(map.year, map.month - 1, map.day, map.hour === 24 ? 0 : map.hour, map.minute, map.second);
  };

  const targetTime = Date.UTC(parseInt(y, 10), parseInt(mo, 10) - 1, parseInt(d, 10), parseInt(h, 10), parseInt(mi, 10), parseInt(s, 10));
  const localGuessTime = getParts(guessDate);
  const diff = targetTime - localGuessTime;
  const correctedDate = new Date(guessDate.getTime() + diff);

  return correctedDate.toISOString().replace(/\.\d{3}Z$/, "Z");
}
