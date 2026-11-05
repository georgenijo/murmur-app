import { expect, test } from '@playwright/test';
import { surfaceCases } from './surface-matrix';
import { STREAMS } from '../src/lib/events';

for (const { appearance, width, height, label } of surfaceCases()) {
}
