import { readFileSync } from 'node:fs';
import { checkFixture } from './check-sidebar-fixtures.mjs';

/** Validate before any host runner observes the shared behavior vectors. */
export const loadSidebarBehavior = (value = JSON.parse(readFileSync(
  new URL('../fixtures/clients/sidebar/behavior-v1.json', import.meta.url), 'utf8',
))) => {
  checkFixture(value);
  return value;
};
