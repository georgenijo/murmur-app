import { invoke } from '@tauri-apps/api/core';

export interface CorpusPrompt {
  id: string;
  label: string;
  category: 'Short' | 'Everyday' | 'Technical' | 'Numbers' | 'Disfluent' | 'Long' | 'Delivery';
  direction: string;
  reference: string;
}

export const PERSONAL_CORPUS_PROMPTS: CorpusPrompt[] = [
  {
    id: 'open-project-dashboard',
    label: 'Short command',
    category: 'Short',
    direction: 'Read naturally.',
    reference: 'Open the project dashboard.',
  },
  {
    id: 'tomorrow-reminder',
    label: 'Short reminder',
    category: 'Short',
    direction: 'Read naturally.',
    reference: 'Create a reminder for tomorrow morning.',
  },
  {
    id: 'package-front-desk',
    label: 'Everyday sentence',
    category: 'Everyday',
    direction: 'Use your ordinary dictation pace.',
    reference: 'The package arrived at the front desk, so I will pick it up after lunch.',
  },
  {
    id: 'weekly-planning',
    label: 'Planning sentence',
    category: 'Everyday',
    direction: 'Use your ordinary dictation pace.',
    reference: 'Let us review the weekly plan, identify the biggest risk, and decide who owns the next step.',
  },
  {
    id: 'customer-follow-up',
    label: 'Customer follow-up',
    category: 'Everyday',
    direction: 'Read as if you were dictating an email.',
    reference: 'Thanks for the thoughtful feedback. I will share an updated proposal by the end of the day.',
  },
  {
    id: 'tauri-capture-worker',
    label: 'Tauri architecture',
    category: 'Technical',
    direction: 'Pronounce the technical terms normally.',
    reference: 'The Tauri application uses a Rust capture worker and a React interface to keep microphone processing local.',
  },
  {
    id: 'npm-typescript-command',
    label: 'Developer command',
    category: 'Technical',
    direction: 'Read every token, including the letters in the flag.',
    reference: 'Run npm install, then use npx tsc dash dash no emit before opening the pull request.',
  },
  {
    id: 'infrastructure-names',
    label: 'Infrastructure jargon',
    category: 'Technical',
    direction: 'Use your normal pronunciation.',
    reference: 'PostgreSQL, Kubernetes, Core ML, and whisper dot cpp all have different runtime characteristics.',
  },
  {
    id: 'invoice-currency',
    label: 'Currency and decimals',
    category: 'Numbers',
    direction: 'Read the numbers as written.',
    reference: 'The invoice total is one thousand two hundred forty-three dollars and sixty-seven cents.',
  },
  {
    id: 'meeting-date-time',
    label: 'Date and time',
    category: 'Numbers',
    direction: 'Read naturally.',
    reference: 'Schedule the design review for Thursday, September seventeenth, at two thirty in the afternoon.',
  },
  {
    id: 'storage-percentage',
    label: 'Units and percentage',
    category: 'Numbers',
    direction: 'Read naturally.',
    reference: 'The archive is two point four gigabytes, and compression reduced its size by thirty-eight percent.',
  },
  {
    id: 'filler-planning',
    label: 'Natural filler words',
    category: 'Disfluent',
    direction: 'Keep the filler words; do not polish the sentence.',
    reference: 'Um, I think we should, you know, move the planning meeting to Friday because the draft is not quite ready.',
  },
  {
    id: 'spoken-correction',
    label: 'Self-correction',
    category: 'Disfluent',
    direction: 'Pause briefly at the dash and correct yourself naturally.',
    reference: 'Send the summary to Sarah—actually, send it to Priya before lunch.',
  },
  {
    id: 'repeated-thought',
    label: 'Repeated phrase',
    category: 'Disfluent',
    direction: 'Keep the repetition.',
    reference: 'The main issue, the main issue is that we need a stable benchmark before we optimize anything else.',
  },
  {
    id: 'release-paragraph',
    label: 'Release paragraph',
    category: 'Long',
    direction: 'Read at a comfortable, steady pace.',
    reference: 'Before we publish the release, we should verify the signature, inspect the performance report, and confirm that every local model still loads correctly. If one stage fails, the report should identify the exact boundary instead of hiding the error behind a generic message.',
  },
  {
    id: 'privacy-paragraph',
    label: 'Privacy paragraph',
    category: 'Long',
    direction: 'Read at a comfortable, steady pace.',
    reference: 'A private voice application should make its data boundaries obvious. Audio can remain on the device, transcripts can be retained only when requested, and diagnostic events can describe timing without storing the words that a person dictated.',
  },
  {
    id: 'benchmark-paragraph',
    label: 'Benchmark paragraph',
    category: 'Long',
    direction: 'Read at a comfortable, steady pace.',
    reference: 'A useful benchmark separates cold startup from warm inference and reports both median and tail latency. Replaying the same recordings removes speaking variation, while several different recordings keep the corpus representative of real work.',
  },
  {
    id: 'deliberate-pauses',
    label: 'Deliberate pauses',
    category: 'Delivery',
    direction: 'Pause for about one second at each ellipsis.',
    reference: 'First we capture the audio... then we process the transcript... and finally we deliver the result.',
  },
  {
    id: 'faster-delivery',
    label: 'Faster delivery',
    category: 'Delivery',
    direction: 'Read this one somewhat faster than your normal pace, while staying clear.',
    reference: 'Reliable automation lets us compare every candidate build quickly without repeating a long manual testing session.',
  },
  {
    id: 'quieter-delivery',
    label: 'Quieter delivery',
    category: 'Delivery',
    direction: 'Read slightly quieter than usual without moving away from the microphone.',
    reference: 'This quieter sample checks whether voice activity detection still preserves a complete sentence.',
  },
];

export interface CorpusRecordingEntry {
  entryId: string;
  promptIndex: number;
  promptId: string;
  label: string;
  reference: string;
  take: number;
  selected: boolean;
  fileName: string;
  sha256: string;
  recordedAt: string;
  sampleRate: number;
  durationMs: number;
  peak: number;
  rms: number;
  clippingPercent: number;
  deviceLabel: string;
  qualityWarnings: string[];
}

export interface CorpusSummary {
  corpusDirectory: string;
  recordings: CorpusRecordingEntry[];
}

export interface CorpusStopResponse {
  corpusDirectory: string;
  recording: CorpusRecordingEntry;
}

export interface CorpusStatusEvent {
  state: 'idle' | 'starting' | 'recording' | 'saving' | 'recovering' | 'error';
  error: string | null;
}

export async function startCorpusRecording(input: {
  promptIndex: number;
  prompt: CorpusPrompt;
  deviceId: string;
  deviceLabel: string;
}): Promise<void> {
  await invoke('start_corpus_recording', {
    request: {
      promptIndex: input.promptIndex,
      promptId: input.prompt.id,
      label: input.prompt.label,
      reference: input.prompt.reference,
      deviceId: input.deviceId,
      deviceLabel: input.deviceLabel,
    },
  });
}

export async function stopCorpusRecording(): Promise<CorpusStopResponse> {
  return invoke('stop_corpus_recording');
}

export async function cancelCorpusRecording(): Promise<boolean> {
  return invoke('cancel_corpus_recording');
}

export async function getCorpusSummary(): Promise<CorpusSummary> {
  return invoke('get_corpus_summary');
}

export async function openCorpusFolder(): Promise<void> {
  await invoke('open_corpus_folder');
}
