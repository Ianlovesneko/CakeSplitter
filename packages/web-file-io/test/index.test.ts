import { describe, expect, it } from 'vitest';

import { DIRECT_FOLDER_MODE_ENABLED, supportsDirectFolderMode } from '../src/index';

describe('browser output capability', () => {
  it('keeps direct folder publication disabled until no-replace semantics are available', () => {
    expect(DIRECT_FOLDER_MODE_ENABLED).toBe(false);
    expect(supportsDirectFolderMode()).toBe(false);
  });
});
