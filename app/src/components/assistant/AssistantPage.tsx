import type { QueryCommandConfig } from '../../lib/queryProviders';
import type { SmartAutoMicrophoneRequest } from '../../lib/settings';
import { useAssistant } from '../../lib/hooks/useAssistant';
import { AssistantWorkspace } from './AssistantWorkspace';
import './assistant.css';

interface Props { command: QueryCommandConfig; deviceName: string | null; smartAuto: SmartAutoMicrophoneRequest | null; voiceAvailable: boolean; requestedId: string | null; onSettings: () => void }
export function AssistantPage(props: Props) {
  const state = useAssistant(props);
  return <AssistantWorkspace
    connected={state.connected} configured={props.command.provider === 'custom' && !!props.command.executable.trim()}
    loading={state.loading} pending={state.pending} conversations={state.conversations} selectedId={state.conversation?.id ?? null}
    messages={(state.conversation?.messages ?? []).map((m) => ({ id: m.id, role: m.role, text: m.content,
      status: m.status === 'ready' ? 'complete' : m.status === 'pending' ? 'running' : m.status === 'interrupted' ? 'cancelled' : m.status }))}
    phase={state.phase} error={state.error} voiceAvailable={props.voiceAvailable} dictation={state.dictation}
    onConnect={state.connect} onDisconnect={state.disconnect} onNew={state.newConversation} onSelect={state.select} onDelete={state.remove}
    onSend={state.send} onVoice={state.voice} onFinishVoice={state.finishVoice} onStop={state.stop} onSettings={props.onSettings}
  />;
}
