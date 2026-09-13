import { useCallback, useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import { assistantConnected, assistantError, createAssistant, getAssistant, isConversationId, listAssistant, type AssistantConversation, type AssistantReceipt, type AssistantSummary } from '../assistant';
import type { QueryCommandConfig } from '../queryProviders';
import type { SmartAutoMicrophoneRequest } from '../settings';
import type { AssistantPhase } from '../../components/assistant/AssistantWorkspace';

interface Options { command: QueryCommandConfig; deviceName: string | null; smartAuto: SmartAutoMicrophoneRequest | null; requestedId?: string | null }

export function useAssistant({ command, deviceName, smartAuto, requestedId }: Options) {
  const [connected, setConnected] = useState(false);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState(false);
  const [listenersReady, setListenersReady] = useState(false);
  const [conversations, setConversations] = useState<AssistantSummary[]>([]);
  const [conversation, setConversation] = useState<AssistantConversation | null>(null);
  const [phase, setPhase] = useState<AssistantPhase>('idle');
  const [error, setError] = useState<string | null>(null);
  const mounted = useRef(true);
  const selected = useRef<string | null>(null);
  const activePass = useRef<number | null>(null);
  const busy = useRef(false);
  const selectionGeneration = useRef(0);
  const hydrationGeneration = useRef(0);
  const lastState = useRef<{ pass: number; phase: AssistantPhase } | null>(null);
  const awaitingAdmission = useRef(false);
  const cancelAdmission = useRef(false);
  const commandKey = JSON.stringify(command);
  const commandRef = useRef(command); commandRef.current = command;

  const hydrate = useCallback(async (id: string) => {
    const generation = selectionGeneration.current;
    const ticket = ++hydrationGeneration.current;
    const result = await getAssistant(id);
    if (mounted.current && selected.current === id && generation === selectionGeneration.current && ticket === hydrationGeneration.current) {
      setConversation(result); activePass.current = result.activePassId;
      if (result.activePassId === null) setPhase('idle');
      else if (result.liveState && ['connecting', 'listening', 'transcribing', 'running'].includes(result.liveState)) setPhase(result.liveState as AssistantPhase);
    }
  }, []);
  const refreshList = useCallback(async () => {
    const result = await listAssistant();
    if (mounted.current) setConversations(result);
    return result;
  }, []);
  const select = useCallback(async (id: string) => {
    if (!mounted.current || !isConversationId(id)) return;
    selectionGeneration.current += 1; selected.current = id;
    setConversation(null); setError(null); setLoading(true);
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
        if (payload.conversationId === selected.current && payload.state) {
          const next = payload.state;
          if (['connecting', 'listening', 'transcribing', 'running'].includes(next)) {
            lastState.current = { pass: payload.queryPassId, phase: next as AssistantPhase };
            setPhase(next as AssistantPhase); activePass.current = payload.queryPassId;
          } else { lastState.current = { pass: payload.queryPassId, phase: 'idle' }; setPhase('idle'); activePass.current = null; }
        }
        void refresh();
      }).then((unlisten) => {
        if (disposed) unlisten();
        else { unlisteners.push(unlisten); if (unlisteners.length === 2) setListenersReady(true); }
      }).catch(() => { if (!disposed) { setListenersReady(false); setError('Could not subscribe to assistant updates. Reopen this page before sending.'); } });
    }
    return () => { disposed = true; unlisteners.forEach((stop) => stop()); };
  }, [hydrate, refreshList]);

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
    setPhase(kind === 'text' ? 'running' : 'connecting');
    let receipt: AssistantReceipt;
    try {
      receipt = await invoke<AssistantReceipt>(kind === 'text' ? 'send_assistant_message' : 'start_assistant_voice', {
        conversationId: id, command: commandRef.current, consent: true,
        ...(kind === 'text' ? { message: text } : { deviceName, smartAuto }),
      });
    } catch (e) { setPhase('idle'); throw e; }
    finally { awaitingAdmission.current = false; }
    if (!isConversationId(receipt?.conversationId) || !Number.isSafeInteger(receipt.queryPassId) || receipt.queryPassId <= 0) throw new Error('invalid_assistant_response');
    if (!mounted.current || cancelAdmission.current) {
      await invoke('cancel_query', { queryPassId: receipt.queryPassId });
      if (mounted.current) { await hydrate(id); await refreshList(); }
      return;
    }
    activePass.current = receipt.queryPassId;
    setPhase(lastState.current?.pass === receipt.queryPassId ? lastState.current.phase : kind === 'text' ? 'running' : 'connecting');
    await hydrate(id); await refreshList();
  }), [deviceName, smartAuto, mutate, hydrate, refreshList, listenersReady]);
  const stop = useCallback(async () => {
    if (awaitingAdmission.current) cancelAdmission.current = true;
    const pass = activePass.current;
    if (pass === null) return;
    try { await invoke('cancel_query', { queryPassId: pass }); if (mounted.current && selected.current) await hydrate(selected.current); }
    catch (e) { if (mounted.current) setError(assistantError(e)); }
  }, [hydrate]);
  const finishVoice = useCallback(async () => {
    const pass = activePass.current;
    if (pass === null) return;
    setPhase('transcribing');
    // The native finish command can remain pending through ASR/inference.
    // Never hold the mutation lock: Stop must stay usable during that work.
    try { await invoke('finish_query_capture', { queryPassId: pass }); }
    catch (e) { if (mounted.current) setError(assistantError(e)); }
  }, []);
  return { connected, loading, pending: pending || (connected && !listenersReady), conversations, conversation, phase, error, select, connect, disconnect, newConversation, remove, stop, finishVoice, send: (text: string) => start('text', text), voice: async () => { await start('voice'); } };
}
