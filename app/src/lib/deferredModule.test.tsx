import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { deferredModule, useDeferredModule, type DeferredModule } from './deferredModule';

type PanelModule = { Panel: () => JSX.Element };
const Panel = () => <p>panel</p>;

let container: HTMLDivElement;
let root: Root;
beforeEach(() => { container = document.createElement('div'); document.body.append(container); root = createRoot(container); });
afterEach(async () => { await act(async () => root.unmount()); container.remove(); });

function Host({ module, retryKey, commits }: { module: DeferredModule<PanelModule>; retryKey?: unknown; commits?: string[] }) {
  const loaded = useDeferredModule(module, retryKey);
  commits?.push(loaded ? 'panel' : 'fallback');
  return loaded ? <loaded.Panel /> : <p>fallback</p>;
}

const flush = () => act(async () => { await new Promise((resolve) => setTimeout(resolve, 0)); });
const texts = () => Array.from(container.querySelectorAll('p'), (p) => p.textContent);

describe('deferredModule', () => {
  it('renders a preloaded module in the first commit, with no fallback frame', async () => {
    const module = deferredModule<PanelModule>(async () => ({ Panel }));
    await module.preload();
    const commits: string[] = [];
    act(() => root.render(<Host module={module} commits={commits} />));
    expect(texts()).toEqual(['panel']);
    expect(commits).not.toContain('fallback');
  });

  it('loads once even when several hosts mount before it resolves', async () => {
    const load = vi.fn(async () => ({ Panel }));
    const module = deferredModule<PanelModule>(load);
    act(() => root.render(<><Host module={module} /><Host module={module} /></>));
    expect(texts()).toEqual(['fallback', 'fallback']);
    await flush();
    expect(texts()).toEqual(['panel', 'panel']);
    expect(load).toHaveBeenCalledTimes(1);
  });

  it('keeps the fallback on a failed load and retries when the retry key changes', async () => {
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const load = vi.fn<() => Promise<PanelModule>>()
      .mockRejectedValueOnce(new Error('chunk failed'))
      .mockResolvedValue({ Panel });
    const module = deferredModule(load);
    act(() => root.render(<Host module={module} retryKey={false} />));
    await flush();
    expect(texts()).toEqual(['fallback']);
    expect(error).toHaveBeenCalled();
    act(() => root.render(<Host module={module} retryKey />));
    await flush();
    expect(texts()).toEqual(['panel']);
    expect(load).toHaveBeenCalledTimes(2);
    error.mockRestore();
  });
});
