// -- WP WEBM4 -- Finding the command palette's entries by what was typed: the desktop app's rules
// (Qt's PaletteController, LAY6), so both find the same entry for the same words.
//
// The query's words must each be in the entry, in any order; case, accents and punctuation between
// words do not count. A word is found, best first, as the start of the title, the start of one of
// its words, inside it, as the first letters of its words ("sg": Signal Generator), or with letters
// left out ("cfeed": Crossfeed). An alias is another title; a keyword (a page's name, a menu's)
// counts less, and only as it is spelt. Of equals a greyed entry comes after, and the shorter title
// first.

export interface Searchable {
  title: string;
  aliases?: readonly string[];
  keywords?: readonly string[];
  enabled: boolean;
}

/** For comparing: no case, no accents. */
export function folded(text: string): string {
  return text.normalize("NFKD").replace(/\p{Mn}/gu, "").toLowerCase();
}

const letter = (c: string | undefined) => c !== undefined && /[\p{L}\p{N}]/u.test(c);

function startsWord(text: string, i: number): boolean {
  return letter(text[i]) && (i === 0 || !letter(text[i - 1]));
}

/** `word` is the first letters of words of `text` that follow each other. */
function isInitials(word: string, text: string): boolean {
  let initials = "";
  for (let i = 0; i < text.length; ++i) if (startsWord(text, i)) initials += text[i];
  return initials.includes(word);
}

/**
 * How many letters of `text` lie between those of `word`, which are there in order from the start
 * of a word on; -1 for too many, or not there.
 */
function lettersBetween(word: string, text: string): number {
  let best = -1;
  for (let start = 0; start < text.length; ++start) {
    if (text[start] !== word[0] || !startsWord(text, start)) continue;
    let at = start;
    let n = 1;
    while (n < word.length && ++at < text.length) if (text[at] === word[n]) ++n;
    if (n < word.length) continue;
    const between = at - start + 1 - word.length;
    if (between <= 2 * word.length && (best < 0 || between < best)) best = between;
  }
  return best;
}

/** One word of the query in a title, 0 to 100. */
function titleScore(word: string, title: string): number {
  if (title.startsWith(word)) return 100;
  let found = 0;
  for (let i = title.indexOf(word); i >= 0; i = title.indexOf(word, i + 1)) {
    if (startsWord(title, i)) return 90;
    found = 70;
  }
  // A single letter is not looked for inside words
  if (word.length < 2 && letter(word[0])) return 0;
  if (found > 0) return found;
  if (isInitials(word, title)) return 65;
  if (word.length < 3) return 0;
  const between = lettersBetween(word, title);
  return between < 0 ? 0 : Math.max(30, 55 - 2 * between);
}

/** One word of the query in a keyword. */
function keywordScore(word: string, keyword: string): number {
  if (keyword === word) return 62;
  for (let i = keyword.indexOf(word); i >= 0; i = keyword.indexOf(word, i + 1)) if (startsWord(keyword, i)) return 60;
  return word.length >= 3 && keyword.includes(word) ? 45 : 0;
}

/** What is looked for: marks alone ("·") only when there is nothing else. */
function words(query: string): string[] {
  const all = folded(query).split(" ").filter(Boolean);
  const out = all.filter((w) => [...w].some(letter));
  return out.length ? out : all;
}

/** How well `query` finds `entry`: 0 not at all. `matched` is the alias or keyword it took, "" for the title. */
export function score(entry: Searchable, query: string): { score: number; matched: string } {
  const typed = words(query);
  if (!typed.length) return { score: 1, matched: "" };
  const whole = typed.join(" ");
  const title = folded(entry.title);
  const aliases = entry.aliases ?? [];
  const keywords = entry.keywords ?? [];
  let sum = 0;
  let by = "";
  let byScore = 0;
  let bySeveral = 0;
  for (const word of typed) {
    let best = titleScore(word, title);
    let from = "";
    for (const alias of aliases) {
      const s = titleScore(word, folded(alias)) - 2;
      if (s > best) {
        best = s;
        from = alias;
      }
    }
    for (const keyword of keywords) {
      const s = keywordScore(word, folded(keyword));
      if (s > best) {
        best = s;
        from = keyword;
      }
    }
    if (best <= 0) return { score: 0, matched: "" };
    sum += best;
    if (!from) continue;
    ++bySeveral;
    if (best > byScore) {
      by = from;
      byScore = best;
    }
  }
  // Rather one that has every word
  if (bySeveral > 1)
    for (const text of [...aliases, ...keywords]) {
      const f = folded(text);
      if (typed.every((w) => f.includes(w))) {
        by = text;
        break;
      }
    }
  let total = sum / typed.length;
  if (title === whole) total += 20;
  else if (title.startsWith(whole)) total += 10;
  else if (title.includes(whole)) total += 5;
  return { score: total, matched: by };
}

/** The entries `query` finds, best first, each with what it was found by; all, in their order, for "". */
export function search<T extends Searchable>(entries: readonly T[], query: string): (T & { matched: string })[] {
  const any = words(query).length > 0;
  const found: { entry: T & { matched: string }; score: number }[] = [];
  for (const entry of entries) {
    let { score: s, matched } = score(entry, query);
    if (s <= 0) continue;
    if (any) {
      if (!entry.enabled) s -= 15;
      // Of equals the shorter title first
      s -= entry.title.length / 100;
    }
    found.push({ entry: { ...entry, matched }, score: s });
  }
  // Array.prototype.sort is stable
  if (any) found.sort((a, b) => b.score - a.score);
  return found.map((f) => f.entry);
}
