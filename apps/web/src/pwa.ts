export interface PwaSnapshot {
  online: boolean;
  serviceWorkerSupported: boolean;
  installed: boolean;
  updateAvailable: boolean;
}

export interface PwaController {
  snapshot(): PwaSnapshot;
  activateUpdate(activeTask: boolean): Promise<void>;
  dispose(): void;
}

export interface WaitingServiceWorker {
  postMessage(message: unknown): void;
}

export function activateWaitingServiceWorker(
  waiting: WaitingServiceWorker | undefined,
  activeTask: boolean,
): boolean {
  if (activeTask) {
    throw new Error('Finish, pause, or cancel active file processing before applying an update.');
  }
  if (!waiting) return false;
  waiting.postMessage({ type: 'ACTIVATE_UPDATE' });
  return true;
}

export async function observePwa(onChange: (snapshot: PwaSnapshot) => void): Promise<PwaController> {
  let registration: ServiceWorkerRegistration | undefined;
  let disposed = false;
  const current = (): PwaSnapshot => ({
    online: navigator.onLine,
    serviceWorkerSupported: 'serviceWorker' in navigator,
    installed: registration?.active !== undefined,
    updateAvailable: registration?.waiting !== undefined && registration.waiting !== null,
  });
  const publish = () => {
    if (!disposed) onChange(current());
  };
  const online = () => publish();
  const offline = () => publish();
  const controllerChange = () => publish();
  window.addEventListener('online', online);
  window.addEventListener('offline', offline);

  if ('serviceWorker' in navigator) {
    registration = await navigator.serviceWorker.register('/sw.js', { scope: '/' });
    registration.addEventListener('updatefound', () => {
      const installing = registration?.installing;
      installing?.addEventListener('statechange', publish);
    });
    navigator.serviceWorker.addEventListener('controllerchange', controllerChange);
  }
  publish();

  return {
    snapshot: current,
    activateUpdate(activeTask: boolean): Promise<void> {
      activateWaitingServiceWorker(registration?.waiting ?? undefined, activeTask);
      return Promise.resolve();
    },
    dispose() {
      disposed = true;
      window.removeEventListener('online', online);
      window.removeEventListener('offline', offline);
      navigator.serviceWorker?.removeEventListener('controllerchange', controllerChange);
    },
  };
}
