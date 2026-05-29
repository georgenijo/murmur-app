import { useState, useEffect, useCallback, useRef } from 'react';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import { transcribeFile } from '../dictation';
import { flog } from '../log';

const AUDIO_EXTENSIONS = ['wav', 'mp3', 'm4a'];
const UNSUPPORTED_MESSAGE = 'Unsupported file type. Use WAV, MP3, or M4A.';

export type FileTranscriptionStatus = 'idle' | 'processing' | 'complete' | 'error';

function hasAudioExtension(path: string): boolean {
  const ext = path.split('.').pop()?.toLowerCase();
  return !!ext && AUDIO_EXTENSIONS.includes(ext);
}

function baseName(path: string): string {
  return path.split(/[\\/]/).pop() || path;
}

interface UseFileTranscriptionProps {
  /** Persist completed transcriptions to shared history (no WPM stats). */
  addEntry: (text: string, duration: number, source?: 'recording' | 'file', sourceName?: string) => void;
}

/**
 * Manages the file-transcription flow: invokes the Rust `transcribe_file`
 * command, tracks status/result/error, and wires window drag-and-drop (which
 * yields real filesystem paths via the Tauri webview event).
 */
export function useFileTranscription({ addEntry }: UseFileTranscriptionProps) {
  const [status, setStatus] = useState<FileTranscriptionStatus>('idle');
  const [result, setResult] = useState('');
  const [error, setError] = useState('');
  const [fileName, setFileName] = useState('');
  const [isDragging, setIsDragging] = useState(false);

  // Ref mirror so the drag-drop listener (registered once) sees current status.
  const statusRef = useRef(status);
  statusRef.current = status;

  const transcribe = useCallback(async (path: string) => {
    if (statusRef.current === 'processing') return;
    if (!hasAudioExtension(path)) {
      setError(UNSUPPORTED_MESSAGE);
      setStatus('error');
      return;
    }
    const name = baseName(path);
    setFileName(name);
    setResult('');
    setError('');
    setStatus('processing');
    flog.info('file-transcribe', 'start', { name: baseName(path) });

    const res = await transcribeFile(path);
    if (res.type === 'error') {
      setError(res.error || 'Transcription failed');
      setStatus('error');
      flog.warn('file-transcribe', 'error', { error: res.error });
      return;
    }

    const text = res.text || '';
    setResult(text);
    setStatus('complete');
    if (text.trim()) {
      addEntry(text, res.duration ?? 0, 'file', name);
    }
    flog.info('file-transcribe', 'complete', { textLen: text.length });
  }, [addEntry]);

  const reset = useCallback(() => {
    setStatus('idle');
    setResult('');
    setError('');
    setFileName('');
  }, []);

  // Drag-and-drop via the Tauri webview — provides absolute file paths
  // (the plain HTML5 drop event does not, for security reasons). Drag-drop is
  // an optional convenience; if the listener can't be registered the picker
  // button still works, so failures degrade gracefully rather than break the UI.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    try {
      getCurrentWebview().onDragDropEvent((event) => {
        const payload = event.payload;
        if (payload.type === 'enter' || payload.type === 'over') {
          setIsDragging(true);
        } else if (payload.type === 'leave') {
          setIsDragging(false);
        } else if (payload.type === 'drop') {
          setIsDragging(false);
          const audioPath = payload.paths.find(hasAudioExtension);
          if (audioPath) {
            void transcribe(audioPath);
          } else if (payload.paths.length > 0) {
            setError(UNSUPPORTED_MESSAGE);
            setStatus('error');
          }
        }
      }).then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      }).catch((e) => {
        flog.warn('file-transcribe', 'drag-drop listener failed', { error: String(e) });
      });
    } catch (e) {
      flog.warn('file-transcribe', 'drag-drop unavailable', { error: String(e) });
    }

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [transcribe]);

  return { status, result, error, fileName, isDragging, transcribe, reset };
}
