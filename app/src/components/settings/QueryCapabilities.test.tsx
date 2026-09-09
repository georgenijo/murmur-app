import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { QueryCapabilities } from './QueryCapabilities';
import type { QueryCommandConfig } from '../../lib/queryProviders';

const invoke = vi.hoisted(() => vi.fn());
vi.mock('@tauri-apps/api/core', () => ({ invoke }));
const command: QueryCommandConfig = { provider: 'claude', executable: '/test/claude', arguments: [], timeoutSeconds: 60, contextLevel: 'none', retainQueryHistory: false };

describe('QueryCapabilities', () => {
  let container: HTMLDivElement;
  let root: Root;
  beforeEach(() => {
    invoke.mockReset();
    container = document.createElement('div');
    document.body.appendChild(container);
    root = createRoot(container);
  });
  afterEach(async () => { await act(async () => root.unmount()); container.remove(); });
  const button = (text: string) => {
    const found = Array.from(container.querySelectorAll('button')).find((node) => node.textContent === text);
    if (!found) throw new Error(`Missing button ${text}`);
    return found;
  };

  it('requires folder selection and a second explicit confirmation, then revokes', async () => {
    invoke.mockImplementation(async (name: string) => {
      if (name === 'choose_query_workspace') return '/public/project';
      if (name === 'confirm_query_workspace') return { profile: 'trusted_read_only', directory: '/public/project' };
      return { profile: 'restricted', directory: null };
    });
    await act(async () => root.render(<QueryCapabilities command={command} />));
    expect(container.textContent).toContain('Capability profile: Restricted');
    await act(async () => button('Choose trusted folder…').click());
    expect(invoke).not.toHaveBeenCalledWith('confirm_query_workspace', expect.anything());
    await act(async () => button('Confirm read-only folder access').click());
    expect(container.textContent).toContain('Capability profile: Trusted read-only');
    await act(async () => button('Revoke access · use Restricted').click());
    expect(container.textContent).toContain('Capability profile: Restricted');
    expect(container.textContent).not.toContain('Trusted folder: /public/project');
  });

  it('disables unsupported providers and never infers a folder', async () => {
    invoke.mockResolvedValue({ profile: 'restricted', directory: null });
    await act(async () => root.render(<QueryCapabilities command={{ ...command, provider: 'codex' }} />));
    expect(button('Choose trusted folder…').disabled).toBe(true);
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it('ignores a selection response after revocation', async () => {
    let finish: (path: string) => void = () => {};
    invoke.mockImplementation((name: string) => name === 'choose_query_workspace'
      ? new Promise<string>((resolve) => { finish = resolve; })
      : Promise.resolve({ profile: 'restricted', directory: null }));
    await act(async () => root.render(<QueryCapabilities command={command} />));
    await act(async () => button('Choose trusted folder…').click());
    await act(async () => button('Revoke access · use Restricted').click());
    await act(async () => finish('/public/project'));
    expect(container.textContent).not.toContain('Confirm read-only folder access');
  });
});
