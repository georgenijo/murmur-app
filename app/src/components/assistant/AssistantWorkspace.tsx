import { useEffect, useRef, useState } from 'react';
import Markdown from 'react-markdown';
import rehypeSanitize from 'rehype-sanitize';

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
}

export type AssistantPhase = 'idle' | 'connecting' | 'listening' | 'transcribing' | 'running';

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
  onConnect: () => Promise<void>;
  onDisconnect: () => Promise<void>;
  onNew: () => Promise<void>;
  onSelect: (id: string) => Promise<void>;
  onDelete: (id: string) => Promise<void>;
  onSend: (text: string) => Promise<boolean>;
  onVoice: () => Promise<void>;
  onFinishVoice: () => Promise<void>;
  onStop: () => Promise<void>;
  onSettings: () => void;
}

function MicrophoneIcon() {
  return <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7"><rect x="9" y="3" width="6" height="12" rx="3" /><path d="M5 10v2a7 7 0 0 0 14 0v-2M12 19v3M8 22h8" /></svg>;
}

const PHASE_LABELS: Record<AssistantPhase, string> = {
  idle: 'Read-only tools · no light controls',
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
  const busy = props.phase !== 'idle' || props.pending;

  useEffect(() => { setDraft(''); setDeleteId(null); }, [props.selectedId]);
  useEffect(() => {
    endRef.current?.scrollIntoView?.({ block: 'nearest', behavior: 'instant' });
  }, [props.messages]);
  useEffect(() => {
    const cancel = (event: KeyboardEvent) => {
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
      <button type="button" className="assistant-new" aria-label="New chat" disabled={busy || props.loading} onClick={() => void props.onNew()}><span aria-hidden="true">＋</span> New chat</button>
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
      <p className="assistant-retention-note">Saved on this Mac and in Pi’s private sessions.</p>
      <button type="button" className="assistant-text-button" disabled={busy} onClick={() => void props.onDisconnect()}>Disconnect assistant</button>
    </aside>
    <div className="assistant-conversation">
      <header className="assistant-conversation-header">
        <div><h2>Pi Assistant</h2><p role="status"><span className="assistant-status-dot" data-busy={busy} />{PHASE_LABELS[props.phase]}</p></div>
        {props.selectedId && <button type="button" className="assistant-text-button" disabled={busy} onClick={() => setDeleteId(props.selectedId)}>Delete chat…</button>}
      </header>
      {deleteId && <div className="assistant-delete-confirm" role="group" aria-label="Confirm local conversation deletion">
        <p>Delete this conversation from Murmur? Pi’s saved session on Ubuntu is not deleted.</p>
        <button type="button" className="assistant-text-button" onClick={() => setDeleteId(null)}>Keep chat</button>
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
          {message.status === 'running' && <span className="assistant-muted">Responding…</span>}
          {message.status === 'cancelled' && <p className="assistant-muted">Stopped. This response may be incomplete.</p>}
          {message.status === 'failed' && <p className="assistant-error">This response did not complete. You can send another message.</p>}
        </article>)}
        <div ref={endRef} />
      </div>
      {props.error && <p role="alert" className="assistant-error assistant-inline-notice">{props.error}</p>}
      <form className="assistant-composer" onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <label className="sr-only" htmlFor="assistant-message">Message Pi Assistant</label>
        <textarea id="assistant-message" ref={composerRef} placeholder="Ask Pi anything…" value={draft} maxLength={32000} disabled={!props.selectedId || busy}
          onChange={(event) => setDraft(event.target.value)} rows={3}
          onKeyDown={(event) => { if (event.key === 'Enter' && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); void submit(); } }} />
        <div className="assistant-composer-actions">
          <p>{props.phase === 'listening' ? 'Your voice will be sent to this conversation.' : 'Enter to send · Shift + Enter for a new line'}</p>
          <div>
            {props.phase === 'idle' && <button type="button" className="assistant-microphone" aria-label="Talk to Pi" title={props.voiceAvailable ? 'Talk to Pi' : 'Microphone and transcription must be ready'} disabled={!props.selectedId || !props.voiceAvailable || busy} onClick={() => void props.onVoice()}><MicrophoneIcon /></button>}
            {props.phase === 'listening' && <button type="button" className="assistant-primary" onClick={() => void props.onFinishVoice()}>Finish speaking</button>}
            {props.phase !== 'idle' && <button type="button" className="assistant-secondary" onClick={() => void props.onStop()}>Stop</button>}
            {props.phase === 'idle' && <button type="submit" className="assistant-primary" disabled={!draft.trim() || !props.selectedId || busy}>Send</button>}
          </div>
        </div>
      </form>
    </div>
  </section>;
}
