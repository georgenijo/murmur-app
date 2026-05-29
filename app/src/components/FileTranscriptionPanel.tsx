import { useState } from 'react';
import { open } from '@tauri-apps/plugin-dialog';
import { useFileTranscription } from '../lib/hooks/useFileTranscription';
import { flog } from '../lib/log';

interface FileTranscriptionPanelProps {
  /** Persist completed transcriptions to shared history. */
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string) => void;
}

export function FileTranscriptionPanel({ addEntry }: FileTranscriptionPanelProps) {
  const { status, result, error, fileName, isDragging, transcribe } = useFileTranscription({ addEntry });
  const [copied, setCopied] = useState(false);

  const processing = status === 'processing';

  const handlePick = async () => {
    try {
      const selected = await open({
        multiple: false,
        filters: [{ name: 'Audio', extensions: ['wav', 'mp3', 'm4a'] }],
      });
      if (typeof selected === 'string') {
        void transcribe(selected);
      }
    } catch (e) {
      flog.warn('file-transcribe', 'file dialog failed', { error: String(e) });
    }
  };

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(result);
      setCopied(true);
      setTimeout(() => setCopied(false), 2000);
    } catch (e) {
      console.error('Failed to copy:', e);
    }
  };

  return (
    <div className="flex-1 flex flex-col overflow-hidden gap-4">
      {/* Drop zone + picker */}
      <div
        className={`shrink-0 rounded-xl border-2 border-dashed p-8 flex flex-col items-center justify-center text-center gap-3 transition-colors ${
          isDragging
            ? 'border-stone-500 bg-stone-100 dark:border-stone-400 dark:bg-stone-800'
            : 'border-stone-300 dark:border-stone-700'
        }`}
      >
        <svg className="w-8 h-8 text-stone-400 dark:text-stone-500" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={1.5} d="M3 16.5v2.25A2.25 2.25 0 005.25 21h13.5A2.25 2.25 0 0021 18.75V16.5m-13.5-9L12 3m0 0l4.5 4.5M12 3v13.5" />
        </svg>
        <div className="text-sm text-stone-600 dark:text-stone-400">
          Drag an audio file here, or
        </div>
        <button
          onClick={handlePick}
          disabled={processing}
          className="px-4 py-2 text-sm font-medium rounded-lg bg-stone-800 text-white hover:bg-stone-700 disabled:opacity-50 disabled:cursor-not-allowed dark:bg-stone-200 dark:text-stone-900 dark:hover:bg-stone-100 transition-colors"
        >
          Choose File
        </button>
        <div className="text-xs text-stone-400 dark:text-stone-500">WAV, MP3, or M4A</div>
      </div>

      {/* Processing indicator */}
      {processing && (
        <div className="shrink-0 flex items-center gap-2 text-sm text-stone-600 dark:text-stone-400">
          <svg className="w-4 h-4 animate-spin" fill="none" viewBox="0 0 24 24">
            <circle className="opacity-25" cx="12" cy="12" r="10" stroke="currentColor" strokeWidth="4" />
            <path className="opacity-75" fill="currentColor" d="M4 12a8 8 0 018-8V0C5.373 0 0 5.373 0 12h4z" />
          </svg>
          <span className="truncate">Transcribing {fileName}…</span>
        </div>
      )}

      {/* Error */}
      {error && (
        <div className="shrink-0 px-4 py-3 bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-lg">
          <p className="text-red-600 dark:text-red-400 text-sm">{error}</p>
        </div>
      )}

      {/* Result */}
      {status === 'complete' && (
        <div className="flex-1 flex flex-col overflow-hidden rounded-xl border border-stone-200 dark:border-stone-700 bg-white dark:bg-stone-800">
          <div className="shrink-0 flex items-center justify-between px-4 py-2 border-b border-stone-200 dark:border-stone-700">
            <span className="text-xs font-medium text-stone-500 dark:text-stone-400 truncate">{fileName}</span>
            <button
              onClick={handleCopy}
              disabled={!result}
              className="text-xs font-medium text-stone-500 hover:text-stone-800 dark:text-stone-400 dark:hover:text-stone-200 disabled:opacity-50 transition-colors"
            >
              {copied ? (
                <span className="text-emerald-600 dark:text-emerald-400">Copied!</span>
              ) : (
                'Copy'
              )}
            </button>
          </div>
          <div className="flex-1 overflow-y-auto p-4 text-sm text-stone-800 dark:text-stone-200 whitespace-pre-wrap break-words">
            {result || <span className="text-stone-400 dark:text-stone-500">No speech detected in this file.</span>}
          </div>
        </div>
      )}
    </div>
  );
}
