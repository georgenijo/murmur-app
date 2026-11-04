import type { Locator } from '@playwright/test';

export async function readableText(locator: Locator) {
  return locator.evaluate(element => {
    // Canvas resolves the browser's actual color syntax, including Tailwind's
    // oklch/color-mix values. Composite every ancestor rather than assuming the
    // fixture's desktop color is the review surface.
    const canvas = document.createElement('canvas');
    canvas.width = canvas.height = 1;
    const context = canvas.getContext('2d', { willReadFrequently: true })!;
    const rgba = (color: string) => {
      context.clearRect(0, 0, 1, 1);
      context.fillStyle = color;
      context.fillRect(0, 0, 1, 1);
      const [r, g, b, a] = context.getImageData(0, 0, 1, 1).data;
      return [r, g, b, a / 255];
    };
    const over = (front: number[], back: number[]) => front.slice(0, 3).map((value, i) => value * front[3] + back[i] * (1 - front[3]));
    const ancestors: Element[] = [];
    for (let node: Element | null = element; node; node = node.parentElement) ancestors.unshift(node);
    let background = [255, 255, 255];
    for (const node of ancestors) background = over(rgba(getComputedStyle(node).backgroundColor), background);
    const style = getComputedStyle(element);
    const foreground = over(rgba(style.color), background);
    const luminance = (color: number[]) => color.map(value => {
      const channel = value / 255;
      return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
    }).reduce((sum, value, i) => sum + value * [0.2126, 0.7152, 0.0722][i], 0);
    const light = luminance(foreground);
    const dark = luminance(background);
    return {
      text: element.textContent, fontSize: style.fontSize, fontWeight: style.fontWeight,
      declaredColor: style.color, foreground, background,
      contrast: (Math.max(light, dark) + 0.05) / (Math.min(light, dark) + 0.05),
    };
  });
}
