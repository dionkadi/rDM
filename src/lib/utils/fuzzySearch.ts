// Fuzzy search utility — used by GlobalSearch and CommandPalette
// Simple, fast, no dependencies

export interface FuzzyMatch<T> {
  item: T;
  score: number;
  matches: number[];
}

export function fuzzySearch<T>(
  items: T[],
  query: string,
  keys: (keyof T | ((item: T) => string))[],
  limit = 50,
): FuzzyMatch<T>[] {
  if (!query.trim()) return items.slice(0, limit).map((item) => ({ item, score: 0, matches: [] }));

  const q = query.toLowerCase();
  const results: FuzzyMatch<T>[] = [];

  for (const item of items) {
    let bestScore = 0;
    let bestMatches: number[] = [];

    for (const key of keys) {
      const str = typeof key === "function" ? key(item) : String(item[key] ?? "");
      const result = scoreString(str, q);
      if (result.score > bestScore) {
        bestScore = result.score;
        bestMatches = result.matches;
      }
    }

    if (bestScore > 0) {
      results.push({ item, score: bestScore, matches: bestMatches });
    }
  }

  results.sort((a, b) => b.score - a.score);
  return results.slice(0, limit);
}

function scoreString(text: string, query: string): { score: number; matches: number[] } {
  const t = text.toLowerCase();
  const matches: number[] = [];

  // Exact substring match wins big
  const idx = t.indexOf(query);
  if (idx >= 0) {
    return { score: 1000 - idx, matches: text.split("").map((_, i) => i).slice(idx, idx + query.length) };
  }

  // Fuzzy character match
  let score = 0;
  let qi = 0;
  let prevMatch = -1;
  let consecutive = 0;

  for (let i = 0; i < t.length && qi < query.length; i++) {
    if (t[i] === query[qi]) {
      matches.push(i);
      // Consecutive bonus
      if (prevMatch === i - 1) {
        consecutive++;
        score += 10 * consecutive;
      } else {
        consecutive = 0;
      }
      // Word start bonus
      if (i === 0 || t[i - 1] === " " || t[i - 1] === "-" || t[i - 1] === "_") {
        score += 20;
      }
      // Case match bonus
      if (text[i] === query[qi]) score += 2;
      score += 1;
      prevMatch = i;
      qi++;
    }
  }

  // Must match all query characters
  if (qi < query.length) {
    return { score: 0, matches: [] };
  }

  return { score, matches };
}

export function highlightMatches(text: string, matches: number[]): string {
  if (matches.length === 0) return escapeHtml(text);
  const set = new Set(matches);
  let out = "";
  let inMatch = false;
  for (let i = 0; i < text.length; i++) {
    const isMatch = set.has(i);
    if (isMatch && !inMatch) {
      out += "<mark>";
      inMatch = true;
    } else if (!isMatch && inMatch) {
      out += "</mark>";
      inMatch = false;
    }
    out += escapeHtml(text[i]);
  }
  if (inMatch) out += "</mark>";
  return out;
}

function escapeHtml(s: string): string {
  return s.replace(/[&<>"']/g, (c) => {
    const map: Record<string, string> = {
      "&": "&amp;",
      "<": "&lt;",
      ">": "&gt;",
      '"': "&quot;",
      "'": "&#39;",
    };
    return map[c] || c;
  });
}