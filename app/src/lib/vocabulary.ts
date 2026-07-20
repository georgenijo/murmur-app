import type { VocabularyEntry, VocabularyScope, VoiceCommand } from './settings';

const BUILTIN_COMMAND_PHRASES = [
  'new paragraph', 'new line', 'scratch that', 'open paren', 'close paren',
  'question mark', 'period', 'comma',
];

export function normalizeVocabularyValue(value: string): string {
  return value.trim().replace(/\s+/g, ' ').toLocaleLowerCase();
}

function scopesOverlap(left: VocabularyScope, right: VocabularyScope): boolean {
  if (left.kind === 'global' || right.kind === 'global') return true;
  if (left.kind === 'app' && right.kind === 'app') return left.bundleId === right.bundleId;
  if (left.kind === 'project' && right.kind === 'project') {
    return left.bundleId === right.bundleId && left.root === right.root;
  }
  return left.bundleId === right.bundleId;
}

export function validateVocabularyEntries(
  entries: VocabularyEntry[],
  voiceCommands: VoiceCommand[] = [],
): string | null {
  if (entries.length > 500) return 'Vocabulary supports at most 500 entries.';
  const enabled = entries.filter((entry) => entry.enabled);
  const commandPhrases = new Set([
    ...BUILTIN_COMMAND_PHRASES,
    ...voiceCommands.map((command) => command.phrase),
  ].map(normalizeVocabularyValue));

  for (const entry of enabled) {
    if (!entry.written.trim()) return 'Every enabled vocabulary entry needs a written form.';
    if (Array.from(entry.written).length > 256) return `The written form '${entry.written.trim()}' is too long.`;
    if (entry.aliases.length > 16) return `'${entry.written.trim()}' supports at most 16 spoken aliases.`;
    if (entry.aliases.some((alias) => !alias.trim())) return `'${entry.written.trim()}' contains an empty spoken alias.`;
    if (entry.aliases.some((alias) => Array.from(alias).length > 256)) {
      return `The spoken alias for '${entry.written.trim()}' is too long.`;
    }
  }

  for (let i = 0; i < enabled.length; i += 1) {
    const left = enabled[i];
    for (let j = i + 1; j < enabled.length; j += 1) {
      const right = enabled[j];
      if (!scopesOverlap(left.scope, right.scope)) continue;
      if (normalizeVocabularyValue(left.written) === normalizeVocabularyValue(right.written)) {
        return `'${right.written.trim()}' conflicts with the existing written term '${left.written.trim()}'.`;
      }
    }
  }

  const edges = enabled.map((entry) => enabled.flatMap((target, index) =>
    scopesOverlap(entry.scope, target.scope)
    && entry.aliases.some((alias) => normalizeVocabularyValue(alias) === normalizeVocabularyValue(target.written))
      ? [index]
      : []));
  const visiting = new Set<number>();
  const visited = new Set<number>();
  const visitsCycle = (node: number): boolean => {
    if (visiting.has(node)) return true;
    if (visited.has(node)) return false;
    visiting.add(node);
    if (edges[node].some(visitsCycle)) return true;
    visiting.delete(node);
    visited.add(node);
    return false;
  };
  if (enabled.some((_, index) => visitsCycle(index))) {
    return 'Cyclic aliases are not allowed. A spoken alias cannot lead back to its starting written term.';
  }

  for (let i = 0; i < enabled.length; i += 1) {
    const entry = enabled[i];
    const seen = new Set<string>();
    for (const alias of entry.aliases) {
      const normalized = normalizeVocabularyValue(alias);
      if (seen.has(normalized)) return `Spoken alias '${alias.trim()}' is duplicated for '${entry.written.trim()}'.`;
      seen.add(normalized);
      if (normalized === normalizeVocabularyValue(entry.written)) {
        return `'${alias.trim()}' is already the written form; remove it from Spoken aliases.`;
      }
      if (commandPhrases.has(normalized)) {
        return `'${alias.trim()}' is a Voice Command phrase. Aliases cannot override commands.`;
      }
      for (let j = 0; j < enabled.length; j += 1) {
        const other = enabled[j];
        if (i === j || !scopesOverlap(entry.scope, other.scope)) continue;
        if (normalized === normalizeVocabularyValue(other.written)) {
          return `'${alias.trim()}' is already the written form for '${other.written.trim()}'.`;
        }
        if (other.aliases.some((candidate) => normalizeVocabularyValue(candidate) === normalized)) {
          return `Spoken alias '${alias.trim()}' is ambiguous between '${entry.written.trim()}' and '${other.written.trim()}'.`;
        }
      }
    }
  }
  return null;
}
