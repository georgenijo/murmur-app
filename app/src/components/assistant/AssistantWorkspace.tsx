import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';
import { ArrowUp, Check, Mic, MoreHorizontal, Plus, Settings2, Square } from 'lucide-react';
import type { AssistantAction } from '../../lib/assistant';
import { AssistantActionCard } from './AssistantActionCard';
import { AssistantActivityMark } from './AssistantActivityMark';

export interface AssistantThreadSummary {
  id: string;
  title: string;
  updatedAtMs: number;
}

export interface AssistantThreadMessage {
  id: string;
  role: 'user' | 'assistant';
  text: string;
  status: 'complete' | 'running' | 'cancelled' | 'failed';
  actions: AssistantAction[];
}

export type AssistantPhase = 'idle' | 'connecting' | 'listening' | 'transcribing' | 'running';
export interface AssistantDictationUpdate { passId: number; text: string; status: 'partial' | 'final' | 'cancelled' }

export interface AssistantWorkspaceProps {
  connected: boolean;
  configured: boolean;
  loading: boolean;
  pending: boolean;
  conversations: AssistantThreadSummary[];
  selectedId: string | null;
  messages: AssistantThreadMessage[];
  phase: AssistantPhase;
  error: string | null;
  voiceAvailable: boolean;
  dictation?: AssistantDictationUpdate | null;
  onConnect: () => Promise<void>;
  onDisconnect: () => Promise<void>;
  onNew: () => Promise<void>;
  onSelect: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onSend: (text: string) => Promise<boolean>;
  onVoice: () => Promise<void>;
  onFinishVoice: () => Promise<void>;
  onStop: () => Promise<void>;
  onConfirmAction: (actionId: string) => Promise<void>;
  onCancelAction: (actionId: string) => Promise<void>;
  onSettings: () => void;
}

function MicrophoneIcon() {
  return <Mic aria-hidden="true" strokeWidth={1.7} />;
}

function dismissDetails(event: KeyboardEvent<HTMLDetailsElement>) {
  if (event.key !== 'Escape' || !event.currentTarget.open) return;
  event.preventDefault();
  event.stopPropagation();
  event.currentTarget.open = false;
  event.currentTarget.querySelector('summary')?.focus();
}

const PHASE_LABELS: Record<AssistantPhase, string> = {
  idle: 'Actions require confirmation',
  connecting: 'Connecting microphone…',
  listening: 'Listening — finish when you’re ready',
  transcribing: 'Transcribing on this Mac…',
  running: 'Pi is working…',
};

export function AssistantWorkspace(props: AssistantWorkspaceProps) {
  const [draft, setDraft] = useState('');
  const [deleteId, setDeleteId] = useState<string | null>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const endRef = useRef<HTMLDivElement>(null);
  const keepChatRef = useRef<HTMLButtonElement>(null);
  const chatOptionsRef = useRef<HTMLElement>(null);
  const voiceBaseRef = useRef('');
  const busy = props.phase !== 'idle' || props.pending;
  const capturing = ['connecting', 'listening', 'transcribing'].includes(props.phase);

  useEffect(() => { setDraft(''); setDeleteId(null); }, [props.selectedId]);
  useEffect(() => {
    const update = props.dictation;
    if (!update) return;
    const prefix = voiceBaseRef.current;
    setDraft(update.status === 'cancelled' ? prefix : `${prefix}${prefix && update.text ? '\n' : ''}${update.text}`);
    if (update.status !== 'partial') composerRef.current?.focus();
  }, [props.dictation]);
  useEffect(() => {
    if (props.dictation?.status === 'partial' && composerRef.current) composerRef.current.scrollTop = composerRef.current.scrollHeight;
  }, [draft, props.dictation]);
  useEffect(() => { if (deleteId) keepChatRef.current?.focus(); }, [deleteId]);
  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: 'nearest', behavior: 'instant' });
  }, [props.messages]);
  useEffect(() => {
    const cancel = (event: globalThis.KeyboardEvent) => {
      if (event.key === 'Escape' && props.phase !== 'idle') {
        event.preventDefault();
        void props.onStop();
      }
    };
    window.addEventListener('keydown', cancel);
    return () => window.removeEventListener('keydown', cancel);
  }, [props.phase, props.onStop]);

  const submit = async () => {
    const value = draft.trim();
    if (!value || busy || !props.selectedId) return;
    if (await props.onSend(value)) setDraft((current) => current === draft ? '' : current);
  };

  if (!props.connected) {
    return <section className="assistant-onboarding" aria-label="Connect your assistant">
      <div className="assistant-welcome-icon" aria-hidden="true"><MicrophoneIcon /></div>
      <h2>A place to think out loud.</h2>
      <p>Talk or type to your Pi assistant. Ask a follow-up, pick up an earlier conversation, or check what’s happening at home.</p>
      <div className="assistant-privacy-card">
        <h3>Your assistant, your connection</h3>
        <p>Uses the Custom executable configured in Voice Query. It must support the Pi Assistant conversation bridge.</p>
        <p>Conversations are saved on this Mac and in Pi’s private sessions on Ubuntu. Pi may send messages and tool results to its model provider. This is separate from Voice Query history.</p>
        <p>Connecting does not grant new tools or permission to control your lights. Ordinary dictation stays local and unchanged.</p>
      </div>
      {props.error && <p role="alert" className="assistant-error">{props.error}</p>}
      <button type="button" className="assistant-primary" disabled={props.pending || props.loading || !props.configured} onClick={() => void props.onConnect()}>
        {props.pending ? 'Connecting…' : 'Connect Pi Assistant'}
      </button>
      {!props.configured && <p>Choose your Pi bridge as the Custom executable in Voice Query first.</p>}
      <button type="button" className="assistant-text-button" onClick={props.onSettings}>Voice Query settings</button>
    </section>;
  }

  return <section className="assistant-workspace" aria-label="Pi Assistant conversations">
    <aside className="assistant-thread-rail" aria-label="Conversations">
      <button type="button" className="assistant-new" aria-label="New chat" disabled={busy || props.loading} onClick={() => void props.onNew()}><Plus aria-hidden="true" /> New chat</button>
      <div className="assistant-thread-list">
        {props.loading && <p role="status" className="assistant-muted">Loading conversations…</p>}
        {!props.loading && props.conversations.length === 0 && <p className="assistant-muted">Your conversations will appear here.</p>}
        {props.conversations.map((thread) => <button
          type="button" key={thread.id} className="assistant-thread" aria-current={props.selectedId === thread.id ? 'page' : undefined}
          disabled={busy} onClick={() => void props.onSelect(thread.id)}
        >
          <span>{thread.title || 'New conversation'}</span>
          <time dateTime={new Date(thread.updatedAtMs).toISOString()}>{new Date(thread.updatedAtMs).toLocaleDateString(undefined, { month: 'short', day: 'numeric' })}</time>
        </button>)}
      </div>
      <details className="assistant-connection" onKeyDown={dismissDetails}>
        <summary><Settings2 aria-hidden="true" /> Connection</summary>
        <div className="assistant-connection-panel">
          <h3>Connected to Pi</h3>
          <p>Saved on this Mac and in Pi’s private sessions. Light changes always require confirmation here.</p>
          <button type="button" className="assistant-text-button" onClick={props.onSettings}>Voice Query settings</button>
          <button type="button" className="assistant-text-button" disabled={busy} onClick={() => void props.onDisconnect()}>Disconnect assistant</button>
        </div>
      </details>
    </aside>
    <div className="assistant-conversation">
      <header className="assistant-conversation-header">
        <div className="assistant-conversation-heading"><h2>Pi Assistant</h2><p role="status"><span className="assistant-status-dot" data-busy={busy} />{PHASE_LABELS[props.phase]}</p></div>
        {props.selectedId && <details className="assistant-chat-options" onKeyDown={dismissDetails}>
          <summary ref={chatOptionsRef} aria-label="Chat options" title="Chat options"><MoreHorizontal aria-hidden="true" /></summary>
          <div className="assistant-chat-options-panel">
            <button type="button" className="assistant-text-button" disabled={busy} onClick={(event) => { setDeleteId(props.selectedId); event.currentTarget.closest('details')?.removeAttribute('open'); }}>Delete chat…</button>
          </div>
        </details>}
      </header>
      {deleteId && <div className="assistant-delete-confirm" role="group" aria-label="Confirm local conversation deletion">
        <p>Delete this conversation from Murmur? Pi’s saved session on Ubuntu is not deleted.</p>
        <button ref={keepChatRef} type="button" className="assistant-text-button" onClick={() => { setDeleteId(null); chatOptionsRef.current?.focus(); }}>Keep chat</button>
        <button type="button" className="assistant-text-button" disabled={busy} onClick={async () => { await props.onDelete(deleteId); setDeleteId(null); }}>Delete from this Mac</button>
      </div>}
      <div className="assistant-message-list" role="log" aria-label="Conversation messages" aria-live="polite" aria-relevant="additions text">
        {props.messages.length === 0 && <div className="assistant-empty">
          <h3>What’s on your mind?</h3>
          <p>{props.selectedId ? 'Type below or use the microphone. Follow-ups stay in this conversation.' : 'Start a new chat to talk with Pi.'}</p>
          {props.selectedId && <div className="assistant-suggestions">
            {['What needs my attention tomorrow?', 'Is the primary bedroom light on?'].map((text) => <button type="button" key={text} disabled={busy} onClick={() => { setDraft(text); composerRef.current?.focus(); }}>{text}</button>)}
          </div>}
        </div>}
        {props.messages.map((message) => <article key={message.id} className="assistant-message" data-role={message.role} aria-label={message.role === 'user' ? 'Your message' : 'Pi response'}>
          <p className="assistant-message-author">{message.role === 'user' ? 'You' : 'Pi'}</p>
          <div className="assistant-message-body">{message.role === 'user'
            ? <p className="assistant-user-text">{message.text}</p>
            : <Markdown rehypePlugins={[rehypeSanitize]} components={{ img: () => null, a: ({ children }) => <span>{children}</span> }}>{message.text}</Markdown>}
          </div>
          {message.actions.map((action) => <AssistantActionCard key={action.action_id} action={action} disabled={busy} onConfirm={props.onConfirmAction} onCancel={props.onCancelAction} />)}
          {message.status === 'running' && <span className="assistant-muted assistant-turn-status"><AssistantActivityMark active />Pi is working…</span>}
          {message.status === 'cancelled' && <p className="assistant-muted">Stopped. This response may be incomplete.</p>}
          {message.status === 'failed' && <p className="assistant-error">This response did not complete. You can send another message.</p>}
        </article>)}
        <div ref={endRef} />
      </div>
      {props.error && <p role="alert" className="assistant-error assistant-inline-notice">{props.error}</p>}
      <form className="assistant-composer" onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <label className="sr-only" htmlFor="assistant-message">Message Pi Assistant</label>
        <textarea id="assistant-message" ref={composerRef} placeholder={capturing ? 'Your words will appear here…' : 'Ask Pi anything…'} value={draft} maxLength={32000} disabled={!props.selectedId || props.phase === 'running'} readOnly={capturing || props.pending}
          onChange={(event) => setDraft(event.target.value)} rows={2}
          onKeyDown={(event) => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void submit(); } }} />
        <div className="assistant-composer-actions">
          <p>{props.phase === 'listening' ? 'Dictating locally · finish to review' : props.phase === 'idle' ? 'Enter to send · Shift + Enter for a new line' : 'Escape to stop'}</p>
          <div>
            {props.phase === 'idle' && <button type="button" className="assistant-microphone" aria-label="Dictate message" title={props.voiceAvailable ? 'Dictate a draft — review before sending' : 'Microphone and transcription must be ready'} disabled={!props.selectedId || !props.voiceAvailable || busy} onClick={() => { voiceBaseRef.current = draft; void props.onVoice(); }}><MicrophoneIcon /></button>}
            {props.phase === 'listening' && <button type="button" className="assistant-primary" onClick={() => void props.onFinishVoice()}><Check aria-hidden="true" />Finish speaking</button>}
            {props.phase !== 'idle' && <button type="button" className="assistant-secondary" onClick={() => void props.onStop()}><Square aria-hidden="true" />Stop</button>}
            {props.phase === 'idle' && <button type="submit" className="assistant-primary assistant-send" aria-label="Send" title="Send message" disabled={!draft.trim() || !props.selectedId || busy}><ArrowUp aria-hidden="true" /></button>}
          </div>
        </div>
      </form>
    </div>
  </section>;
}
