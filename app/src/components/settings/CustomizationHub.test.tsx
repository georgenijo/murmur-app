import { act } from 'react';
import { createRoot, type Root } from 'react-dom/client';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { CustomizationHub } from './CustomizationHub';

let container: HTMLDivElement;
let root: Root;

beforeEach(async () => {
  container = document.createElement('div');
  document.body.appendChild(container);
  root = createRoot(container);
  await act(async () => root.render(<CustomizationHub focusDestination={null} onOpen={vi.fn()} />));
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
});

it('retains list semantics for unstyled customization destinations', () => {
  const list = container.querySelector('[role="list"][aria-label="Customization destinations"]');
  expect(list).not.toBeNull();
  expect(list?.querySelectorAll(':scope > [role="listitem"]')).toHaveLength(4);
});
