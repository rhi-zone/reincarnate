/** GML string functions. */

export function string_length(s: string): number { return s.length; }

export function string_copy(s: string, index: number, count: number): string {
  return s.slice(index - 1, index - 1 + count);
}

export function string_insert(sub: string, s: string, index: number): string {
  return s.slice(0, index - 1) + sub + s.slice(index - 1);
}

export function string_replace_all(content: string, replacee: string, replacer: string): string {
  return content.split(replacee).join(replacer);
}

export function string_lower(s: string): string { return s.toLowerCase(); }
export function string_upper(s: string): string { return s.toUpperCase(); }

export function string_char_at(s: string, index: number): string { return s.charAt(index - 1); }
export function string_pos(sub: string, s: string): number { return s.indexOf(sub) + 1; }
export function string_delete(s: string, index: number, count: number): string {
  return s.slice(0, index - 1) + s.slice(index - 1 + count);
}
export function string_count(sub: string, s: string): number {
  return s.split(sub).length - 1;
}
export function string_trim(s: string, chars?: string[]): string {
  if (!chars || chars.length === 0) return s.trim();
  const set = new Set(chars.join(""));
  let start = 0, end = s.length;
  while (start < end && set.has(s[start])) start++;
  while (end > start && set.has(s[end - 1])) end--;
  return s.slice(start, end);
}

export function string_hash_to_newline(s: string): string { return s.replace(/#/g, "\n"); }

export function string_ord_at(str: string, pos: number): number { return str.charCodeAt(pos - 1) || 0; }

// TODO: re-verify against spcs
export function string_repeat(str: string, count: number): string { return str.repeat(count); }

// TODO: re-verify against spcs
export function string_replace(str: string, sub: string, rep: string): string { return str.replace(sub, rep); }

// TODO: re-verify against spcs
export function string_byte_at(str: string, index: number): number { return str.charCodeAt(index - 1); }

// TODO: re-verify against spcs
export function ord(str: string): number { return str.charCodeAt(0); }

// TODO: re-verify against spcs
export function chr(code: number): string { return String.fromCharCode(code); }

// GML's string() is just String() — emitted code calls String() directly.
