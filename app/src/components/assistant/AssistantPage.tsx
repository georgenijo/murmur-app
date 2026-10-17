import { useMemo } from 'react';
import type { QueryCommandConfig } from '../../lib/queryProviders';
import type { SmartAutoMicrophoneRequest } from '../../lib/settings';
import { useAssistant } from '../../lib/hooks/useAssistant';
import { AssistantWorkspace, type AssistantThreadMessage } from './AssistantWorkspace';
import './assistant.css';

interface Props { command: QueryCommandConfig; deviceName: string | null; smartAuto: SmartAutoMicrophoneRequest | null; voiceAvailable: boolean; requestedId: string | null; onSettings: () => void }
export function AssistantPage(props: Props) {
  const state = useAssistant(props);
  const sourceMessages = state.conversation?.messages;
  const messages = useMemo<AssistantThreadMessage[]>(() => (sourceMessages ?? []).map((m) => ({
    id: m.id, role: m.role, text: m.content, actions: m.actions,
    status: m.status === 'ready' ? 'complete' : m.status === 'pending' ? 'running' : m.status === 'interrupted' ? 'cancelled' : m.status,
  })), [sourceMessages]);
  return <AssistantWorkspace
    connected={state.connected} configured={props.command.provider === 'custom' && !!props.command.executable.trim()}
    loading={state.loading} pending={state.pending} conversations={state.conversations} selectedId={state.conversation?.id ?? null}
    confirmationOutcomeUnknownActionIds={state.conversation?.confirmationOutcomeUnknownActionIds ?? []}
    messages={messages}
    phase={state.phase} error={state.error} voiceAvailable={props.voiceAvailable} dictation={state.dictation}
    onConnect={state.connect} onDisconnect={state.disconnect} onNew={state.newConversation} onSelect={state.select} onDelete={state.remove}
    onSend={state.send} onVoice={state.voice} onFinishVoice={state.finishVoice} onStop={state.stop}
    onConfirmAction={state.confirmAction} onCancelAction={state.cancelAction} onSettings={props.onSettings}
    onRefreshAction={state.refreshAction}
  />;
}
