import { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';

interface LogViewerProps {
  isOpen: boolean;
  onClose: () => void;
}

interface ParsedLine {
  timestamp: string;
  level: string;
  message: string;
}

function parseLine(raw: string): ParsedLine | null {
  const match = raw.match(/^(\S+Z)\s+\[(\w+)\]\s+(.*)$/);
  if (!match) return null;
  return { timestamp: match[1], level: match[2], message: match[3] };
}

const LEVEL_COLORS: Record<string, string> = {
  INFO:  'bg-stone-200 text-stone-700 dark:bg-stone-700 dark:text-stone-300',
  WARN:  'bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-400',
  ERROR: 'bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-400',
};

export function LogViewer({ isOpen, onClose }: LogViewerProps) {
  const [rawLines, setRawLines] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');

  useEffect(() => {
    if (!isOpen) return;
    setLoading(true);
    setError('');
    invoke<string>('get_log_contents', { lines: 200 })
      .then(raw => setRawLines(raw ? raw.split('\n') : []))
      .catch(e => setError(String(e)))
      .finally(() => setLoading(false));
  }, [isOpen]);

  const handleClear = async () => {
    await invoke('clear_logs');
    setRawLines([]);
  };

  const handleCopyAll = () => {
    navigator.clipboard.writeText(rawLines.join('\n'));
  };

  if (!isOpen) return null;

  return (
    <>
      <div className="fixed inset-0 bg-stone-900/50 z-50" onClick={onClose} />
      <div className="fixed inset-0 flex items-center justify-center z-50 pointer-events-none">
        <div className="bg-white dark:bg-stone-800 rounded-2xl shadow-xl w-[600px] max-h-[80vh] flex flex-col pointer-events-auto">
          {/* Header */}
          <div className="flex items-center justify-between px-4 py-3 border-b border-stone-200 dark:border-stone-700 shrink-0">
            <h2 className="text-sm font-semibold text-stone-900 dark:text-stone-100">App Logs</h2>
            <div className="flex items-center gap-2">
              <button
                onClick={handleClear}
                className="px-3 py-1 rounded-md text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors"
              >
                Clear
              </button>
              <button
                onClick={handleCopyAll}
                className="px-3 py-1 rounded-md text-xs font-medium border border-stone-300 dark:border-stone-600 bg-white dark:bg-stone-700 text-stone-700 dark:text-stone-300 hover:bg-stone-50 dark:hover:bg-stone-600 transition-colors"
              >
                Copy All
              </button>
              <button
                onClick={onClose}
                className="p-1 rounded-md hover:bg-stone-100 dark:hover:bg-stone-700 transition-colors"
              >
                <svg className="w-4 h-4 text-stone-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                  <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
                </svg>
              </button>
            </div>
          </div>
          {/* Body */}
          <div className="flex-1 overflow-y-auto p-3 font-mono text-xs bg-stone-50 dark:bg-stone-900 rounded-b-2xl">
            {loading && (
              <p className="text-stone-400 dark:text-stone-500 text-center py-4">Loading…</p>
            )}
            {!loading && error && (
              <p className="text-red-500 text-center py-4">{error}</p>
            )}
            {!loading && !error && rawLines.length === 0 && (
              <p className="text-stone-400 dark:text-stone-500 text-center py-4">No log entries.</p>
            )}
            {!loading && !error && rawLines.map((raw, i) => {
              const p = parseLine(raw);
              if (!p) return null;
              return (
                <div key={i} className="flex items-baseline gap-2 py-0.5 border-b border-stone-100 dark:border-stone-800 last:border-0">
                  <span className="text-stone-400 dark:text-stone-500 shrink-0 tabular-nums">{p.timestamp}</span>
                  <span className={`inline-flex items-center px-1.5 py-0.5 rounded text-[10px] font-semibold shrink-0 ${LEVEL_COLORS[p.level] ?? LEVEL_COLORS['INFO']}`}>
                    {p.level}
                  </span>
                  <span className="text-stone-700 dark:text-stone-300 break-all">{p.message}</span>
                </div>
              );
            })}
          </div>
        </div>
      </div>
    </>
  );
}
