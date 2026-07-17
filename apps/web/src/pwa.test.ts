import { describe, expect, it, vi } from 'vitest';

import { activateWaitingServiceWorker } from './pwa';

describe('controlled service-worker activation', () => {
  it('does not activate a stale update while file processing is active', () => {
    const postMessage = vi.fn();
    expect(() => activateWaitingServiceWorker({ postMessage }, true)).toThrow(/active file processing/u);
    expect(postMessage).not.toHaveBeenCalled();
  });

  it('activates only an explicit waiting worker with an exact command', () => {
    const postMessage = vi.fn();
    expect(activateWaitingServiceWorker(undefined, false)).toBe(false);
    expect(activateWaitingServiceWorker({ postMessage }, false)).toBe(true);
    expect(postMessage).toHaveBeenCalledExactlyOnceWith({ type: 'ACTIVATE_UPDATE' });
  });
});
