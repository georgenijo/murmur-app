import { expect, type Locator } from '@playwright/test';

/**
 * Every selected-segment highlight must be concentric with its track: the gap
 * from the track's outer edge to the highlight equals the difference between
 * their effective corner radii, so the ends of the highlight follow the track.
 */
export async function expectConcentricSegments(track: Locator) {
  const measured = await track.evaluate((element) => {
    const effective = (node: Element, corner: 'TopLeft' | 'TopRight' | 'BottomLeft' | 'BottomRight') => {
      const rect = node.getBoundingClientRect();
      return Math.min(parseFloat(getComputedStyle(node)[`border${corner}Radius`]), rect.height / 2, rect.width / 2);
    };
    const outer = element.getBoundingClientRect();
    return Array.from(element.querySelectorAll(':scope > .settings-segmented-btn'), (button) => {
      const rect = button.getBoundingClientRect();
      const left = rect.left - outer.left;
      const right = outer.right - rect.right;
      const top = rect.top - outer.top;
      const bottom = outer.bottom - rect.bottom;
      return {
        left, right, top, bottom,
        // A highlight that overflows or leaves uneven gaps cannot be concentric.
        trackTopLeft: effective(element, 'TopLeft'), trackTopRight: effective(element, 'TopRight'),
        trackBottomLeft: effective(element, 'BottomLeft'), trackBottomRight: effective(element, 'BottomRight'),
        buttonTopLeft: effective(button, 'TopLeft'), buttonTopRight: effective(button, 'TopRight'),
        buttonBottomLeft: effective(button, 'BottomLeft'), buttonBottomRight: effective(button, 'BottomRight'),
      };
    });
  });
  expect(measured.length).toBeGreaterThan(1);
  const first = measured[0];
  const last = measured[measured.length - 1];
  // The two end segments touch the track's rounded ends.
  expect(first.top).toBeCloseTo(first.bottom, 1);
  expect(first.left).toBeCloseTo(first.top, 1);
  expect(last.right).toBeCloseTo(last.top, 1);
  expect(first.top).toBeGreaterThan(0);
  expect(first.buttonTopLeft).toBeCloseTo(first.trackTopLeft - first.left, 1);
  expect(first.buttonBottomLeft).toBeCloseTo(first.trackBottomLeft - first.left, 1);
  expect(last.buttonTopRight).toBeCloseTo(last.trackTopRight - last.right, 1);
  expect(last.buttonBottomRight).toBeCloseTo(last.trackBottomRight - last.right, 1);
}
