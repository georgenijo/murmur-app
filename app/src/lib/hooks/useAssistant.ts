import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { assistantConnected, assistantError, createAssistant, getAssistant, isConversationId, listAssistant, type AssistantConversation, type AssistantReceipt, type AssistantSummary } from '../assistant';
import type { QueryCommandConfig } from '../queryProviders';
import type { SmartAutoMicrophoneRequest } from '../settings';
import type { AssistantDictationUpdate, AssistantPhase } from '../../components/assistant/AssistantWorkspace';

interface Options { command: QueryCommandConfig; deviceName: string | null; smartAuto: SmartAutoMicrophoneRequest | null; requestedId?: string | null }
interface DraftEvent { conversationId: string; queryPassId: number; state?: string; errorCode?: string; text?: string }

export function useAssistant({ command, deviceName, smartAuto, requestedId }: Options) {
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState(false);
  const [listenersReady, setListenersReady] = useState(false);
  const [conversations, setConversations] = useState<AssistantSummary[]>([]);
  const [conversation, setConversation] = useState<AssistantConversation | null>(null);
  const [phase, setPhase] = useState<AssistantPhase>('idle');
  const [error, setError] = useState<string | null>(null);
  const [dictation, setDictation] = useState<AssistantDictationUpdate | null>(null);
  const mounted = useRef(true);
  const selected = useRef<string | null>(null);
  const activePass = useRef<number | null>(null);
  const busy = useRef(false);
  const selectionGeneration = useRef(0);
  const hydrationGeneration = useRef(0);
  const lastState = useRef<{ pass: number; phase: AssistantPhase } | null>(null);
  const awaitingAdmission = useRef(false);
  const cancelAdmission = useRef(false);
  const draftPass = useRef<number | null>(null);
  const awaitingDraft = useRef(false);
  const bufferedDraft = useRef<DraftEvent[]>([]);
  const finishingDraft = useRef<number | null>(null);
  const cancellingDraft = useRef<number | null>(null);
  const recoveryAttempts = useRef(new Map<string, number>());
  const activeActionOperation = useRef<{
    commandName: 'confirm_assistant_action' | 'cancel_assistant_action' | 'refresh_assistant_action';
    actionId: string;
    passId: number | null;
  } | null>(null);
  const commandKey = JSON.stringify(command);
  const commandRef = useRef(command); commandRef.current = command;

  const hydrate = useCallback(async (id: string) => {
    const generation = selectionGeneration.current;
    const ticket = ++hydrationGeneration.current;
    const result = await getAssistant(id);
    if (mounted.current && selected.current === id && generation === selectionGeneration.current && ticket === hydrationGeneration.current) {
      setConversation(result);
      if (draftPass.current === null && !awaitingDraft.current) {
        activePass.current = result.activePassId;
        if (result.activePassId === null) setPhase('idle');
        else if (result.liveState && ['connecting', 'listening', 'transcribing', 'running'].includes(result.liveState)) setPhase(result.liveState as AssistantPhase);
      }
    }
  }, []);
  const refreshList = useCallback(async () => {
    const result = await listAssistant();
    if (mounted.current) setConversations(result);
    return result;
  }, []);
  const select = useCallback(async (id: string) => {
    if (!mounted.current || !isConversationId(id)) return;
    if (awaitingAdmission.current) cancelAdmission.current = true;
    if (draftPass.current !== null) {
      const pass = draftPass.current;
      draftPass.current = null; finishingDraft.current = null;
      await invoke('cancel_query', { queryPassId: pass });
      activePass.current = null; setPhase('idle');
    }
    selectionGeneration.current += 1; selected.current = id;
    setConversation(null); setError(null); setLoading(true); setDictation(null);
    try { await hydrate(id); } catch (e) { if (mounted.current) setError(assistantError(e)); }
    finally { if (mounted.current && selected.current === id) setLoading(false); }
  }, [hydrate]);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false; selectionGeneration.current += 1;
      if (activePass.current !== null) void invoke('cancel_query', { queryPassId: activePass.current }).catch(() => {});
    };
  }, []);

  useEffect(() => {
    let disposed = false;
    setLoading(true); setError(null);
    void assistantConnected(commandRef.current).then(async (enabled) => {
      if (disposed) return;
      setConnected(enabled);
      if (!enabled) { setConversations([]); setConversation(null); return; }
      const rows = await refreshList();
      if (!disposed && !selected.current && rows.length) await select(rows[0].id);
    }).catch((e) => { if (!disposed) setError(assistantError(e)); }).finally(() => { if (!disposed) setLoading(false); });
    return () => { disposed = true; };
  }, [commandKey, refreshList, select]);

  useEffect(() => { if (requestedId) void select(requestedId); }, [requestedId, select]);

  const applyDraftEvent = useCallback((payload: DraftEvent) => {
    if (!mounted.current || payload.conversationId !== selected.current || payload.queryPassId !== draftPass.current) return;
    if (typeof payload.text === 'string') {
      if (finishingDraft.current === payload.queryPassId || cancellingDraft.current === payload.queryPassId) return;
      setDictation({ passId: payload.queryPassId, text: payload.text, status: 'partial' });
    }
    if (payload.state && ['connecting', 'listening', 'transcribing'].includes(payload.state) && finishingDraft.current !== payload.queryPassId) setPhase(payload.state as AssistantPhase);
    if (payload.state === 'failed' || payload.state === 'cancelled' || (payload.state === 'idle' && payload.errorCode === 'cancelled')) {
      setDictation({ passId: payload.queryPassId, text: '', status: 'cancelled' });
      draftPass.current = null; activePass.current = null; finishingDraft.current = null; cancellingDraft.current = null; setPhase('idle');
      if (payload.state === 'failed') setError(assistantError(payload.errorCode ?? 'transcription_failed'));
    }
    // Final text arrives through finish_assistant_dictation. Its completion,
    // not an earlier terminal event, makes the composer editable again.
  }, []);

  useEffect(() => {
    let disposed = false;
    setListenersReady(false);
    const unlisteners: Array<() => void> = [];
    let refreshQueued = false;
    let refreshNeeded = false;
    const refresh = async () => {
      refreshNeeded = true;
      if (refreshQueued) return;
      refreshQueued = true;
      try {
        while (refreshNeeded && !disposed) {
          refreshNeeded = false;
          if (selected.current) await hydrate(selected.current);
          await refreshList();
        }
      } catch (e) { if (!disposed) setError(assistantError(e)); }
      finally { refreshQueued = false; }
    };
    for (const name of ['assistant-conversation-changed', 'assistant-state-changed']) {
      void listen<{ conversationId: string; queryPassId: number; state?: string }>(name, ({ payload }) => {
        if (disposed || !isConversationId(payload?.conversationId)) return;
        if (payload.conversationId === selected.current && payload.state && draftPass.current === null && !awaitingDraft.current) {
          const next = payload.state;
          if (['connecting', 'listening', 'transcribing', 'running'].includes(next)) {
            lastState.current = { pass: payload.queryPassId, phase: next as AssistantPhase };
            setPhase(next as AssistantPhase); activePass.current = payload.queryPassId;
          } else {
            const operation = activeActionOperation.current;
            if (operation && (operation.passId === null || operation.passId === payload.queryPassId)) {
              if (next === 'failed' && operation.commandName !== 'refresh_assistant_action') {
                recoveryAttempts.current.set(operation.actionId, 0);
              }
              activeActionOperation.current = null;
            }
            lastState.current = { pass: payload.queryPassId, phase: 'idle' }; setPhase('idle'); activePass.current = null;
          }
        }
        void refresh();
      }).then((unlisten) => {
        if (disposed) unlisten();
        else { unlisteners.push(unlisten); if (unlisteners.length === 4) setListenersReady(true); }
      }).catch(() => { if (!disposed) { setListenersReady(false); setError('Could not subscribe to assistant updates. Reopen this page before sending.'); } });
    }
    for (const name of ['assistant-draft-state', 'assistant-draft-partial']) {
      void listen<DraftEvent>(name, ({ payload }) => {
        if (disposed || !isConversationId(payload?.conversationId) || !Number.isSafeInteger(payload.queryPassId) || payload.queryPassId <= 0 || payload.conversationId !== selected.current) return;
        if (awaitingDraft.current) { bufferedDraft.current = [...bufferedDraft.current, payload].slice(-8); return; }
        applyDraftEvent(payload);
      }).then((unlisten) => {
        if (disposed) unlisten();
        else { unlisteners.push(unlisten); if (unlisteners.length === 4) setListenersReady(true); }
      }).catch(() => { if (!disposed) { setListenersReady(false); setError('Could not subscribe to assistant updates. Reopen this page before sending.'); } });
    }
    return () => { disposed = true; unlisteners.forEach((stop) => stop()); };
  }, [hydrate, refreshList, applyDraftEvent]);

  const mutate = useCallback(async (action: () => Promise<void>) => {
    if (busy.current) return false;
    busy.current = true; setPending(true); setError(null);
    try { await action(); return true; }
    catch (e) { if (mounted.current) setError(assistantError(e)); return false; }
    finally { busy.current = false; if (mounted.current) setPending(false); }
  }, []);
  const newConversation = useCallback(async () => {
    await mutate(async () => { const result = await createAssistant(); await refreshList(); await select(result.id); });
  }, [mutate, refreshList, select]);
  const connect = useCallback(async () => {
    await mutate(async () => {
      await invoke('connect_assistant', { command: commandRef.current, consent: true });
      if (!mounted.current) return;
      setConnected(true);
      const rows = await refreshList();
      const id = rows[0]?.id ?? (await createAssistant()).id;
      await refreshList(); await select(id);
    });
  }, [mutate, refreshList, select]);
  const disconnect = useCallback(async () => {
    await mutate(async () => {
      await invoke('disconnect_assistant');
      setConnected(false); selected.current = null; setConversation(null); setConversations([]);
    });
  }, [mutate]);
  const remove = useCallback(async (id: string) => {
    await mutate(async () => {
      await invoke('delete_assistant_conversation', { conversationId: id });
      if (selected.current === id) { selected.current = null; selectionGeneration.current += 1; setConversation(null); }
      const rows = await refreshList(); if (rows[0]) await select(rows[0].id);
    });
  }, [mutate, refreshList, select]);
  const start = useCallback(async (kind: 'text' | 'voice', text?: string) => mutate(async () => {
    if (!listenersReady) throw new Error('assistant_listeners_unavailable');
    const id = selected.current;
    if (!id) throw new Error('not_configured');
    awaitingAdmission.current = true; cancelAdmission.current = false;
    awaitingDraft.current = kind === 'voice'; bufferedDraft.current = []; cancellingDraft.current = null; setDictation(null);
    setPhase(kind === 'text' ? 'running' : 'connecting');
    let receipt: AssistantReceipt;
    try {
      receipt = await invoke<AssistantReceipt>(kind === 'text' ? 'send_assistant_message' : 'start_assistant_dictation', {
        conversationId: id, command: commandRef.current, consent: true,
        ...(kind === 'text' ? { message: text } : { deviceName, smartAuto }),
      });
    } catch (e) { setPhase('idle'); throw e; }
    finally { awaitingAdmission.current = false; awaitingDraft.current = false; }
    if (!isConversationId(receipt?.conversationId) || !Number.isSafeInteger(receipt.queryPassId) || receipt.queryPassId <= 0) throw new Error('invalid_assistant_response');
    if (!mounted.current || cancelAdmission.current || selected.current !== id) {
      await invoke('cancel_query', { queryPassId: receipt.queryPassId });
      if (mounted.current) { setPhase('idle'); if (kind === 'text' && selected.current === id) { await hydrate(id); await refreshList(); } }
      return;
    }
    activePass.current = receipt.queryPassId;
    if (kind === 'voice') {
      draftPass.current = receipt.queryPassId; setPhase('connecting');
      for (const payload of bufferedDraft.current) applyDraftEvent(payload);
      bufferedDraft.current = [];
      return;
    }
    setPhase(lastState.current?.pass === receipt.queryPassId ? lastState.current.phase : kind === 'text' ? 'running' : 'connecting');
    await hydrate(id); await refreshList();
  }), [deviceName, smartAuto, mutate, hydrate, refreshList, listenersReady, applyDraftEvent]);
  const stop = useCallback(async () => {
    if (awaitingAdmission.current) cancelAdmission.current = true;
    const pass = activePass.current;
    if (pass === null) return;
    const wasDraft = draftPass.current === pass;
    if (wasDraft) cancellingDraft.current = pass;
    try {
      await invoke('cancel_query', { queryPassId: pass });
      if (mounted.current) {
        if (wasDraft && draftPass.current === pass) { draftPass.current = null; activePass.current = null; finishingDraft.current = null; cancellingDraft.current = null; setDictation({ passId: pass, text: '', status: 'cancelled' }); setPhase('idle'); }
        else if (selected.current) await hydrate(selected.current);
      }
    }
    catch (e) { if (mounted.current) setError(assistantError(e)); }
  }, [hydrate]);
  const finishVoice = useCallback(async () => {
    const pass = draftPass.current;
    const id = selected.current;
    if (pass === null || finishingDraft.current !== null) return;
    finishingDraft.current = pass;
    setPhase('transcribing');
    // Local ASR only. Never send a message here; the user edits and submits it.
    // Stop remains available while final decoding is pending.
    try {
      const result = await invoke<{ text: string }>('finish_assistant_dictation', { queryPassId: pass });
      if (!mounted.current || draftPass.current !== pass || selected.current !== id || cancellingDraft.current === pass) return;
      if (typeof result?.text !== 'string') throw new Error('transcription_failed');
      setDictation({ passId: pass, text: result.text, status: 'final' });
      draftPass.current = null; activePass.current = null; setPhase('idle');
    } catch (e) {
      if (mounted.current && draftPass.current === pass && selected.current === id && cancellingDraft.current !== pass) {
        setError(assistantError(e)); setDictation({ passId: pass, text: '', status: 'cancelled' });
        draftPass.current = null; activePass.current = null; setPhase('idle');
      }
    } finally { if (finishingDraft.current === pass) finishingDraft.current = null; }
  }, []);
  const operateAction = useCallback(async (commandName: 'confirm_assistant_action' | 'cancel_assistant_action' | 'refresh_assistant_action', actionId: string) => {
    return mutate(async () => {
      if (!listenersReady || !isConversationId(actionId)) throw new Error('invalid_assistant_action');
      const id = selected.current;
      if (!id) throw new Error('assistant_conversation_unavailable');
      setPhase('running');
      const operation: {
        commandName: 'confirm_assistant_action' | 'cancel_assistant_action' | 'refresh_assistant_action';
        actionId: string;
        passId: number | null;
      } = { commandName, actionId, passId: null };
      activeActionOperation.current = operation;
      let receipt: AssistantReceipt;
      try {
        receipt = await invoke<AssistantReceipt>(commandName, { actionId });
      } catch (actionError) {
        activeActionOperation.current = null;
        setPhase('idle');
        throw actionError;
      }
      if (!isConversationId(receipt?.conversationId) || receipt.conversationId !== id
        || !Number.isSafeInteger(receipt.queryPassId) || receipt.queryPassId <= 0) {
        activeActionOperation.current = null;
        throw new Error('invalid_assistant_response');
      }
      operation.passId = receipt.queryPassId;
      activePass.current = receipt.queryPassId;
      setPhase(lastState.current?.pass === receipt.queryPassId ? lastState.current.phase : 'running');
      await hydrate(id);
      await refreshList();
    });
  }, [hydrate, listenersReady, mutate, refreshList]);
  useEffect(() => {
    if (!conversation || phase !== 'idle' || pending || !listenersReady) return;
    const recoverable = conversation.messages
      .flatMap((message) => message.actions)
      .find((action) => {
        const attempts = recoveryAttempts.current.get(action.action_id) ?? 0;
        const limit = action.status === 'executing' ? 3 : action.status === 'proposed' ? 1 : 0;
        return attempts < limit;
      });
    if (!recoverable) return;
    const attempts = recoveryAttempts.current.get(recoverable.action_id) ?? 0;
    recoveryAttempts.current.set(recoverable.action_id, attempts + 1);
    const timer = window.setTimeout(() => {
      void operateAction('refresh_assistant_action', recoverable.action_id);
    }, attempts === 0 ? 0 : 750);
    return () => window.clearTimeout(timer);
  }, [conversation, listenersReady, operateAction, pending, phase]);
  return {
    connected, loading, pending: pending || (connected && !listenersReady), conversations, conversation, phase, error, dictation,
    select, connect, disconnect, newConversation, remove, stop, finishVoice,
    send: (text: string) => start('text', text), voice: async () => { await start('voice'); },
    confirmAction: async (actionId: string) => {
      const accepted = await operateAction('confirm_assistant_action', actionId);
      if (!accepted) {
        recoveryAttempts.current.set(actionId, 1);
        await operateAction('refresh_assistant_action', actionId);
      }
    },
    cancelAction: async (actionId: string) => { await operateAction('cancel_assistant_action', actionId); },
    refreshAction: async (actionId: string) => {
      recoveryAttempts.current.delete(actionId);
      await operateAction('refresh_assistant_action', actionId);
    },
  };
}
