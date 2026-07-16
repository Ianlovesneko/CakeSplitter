import { IncrementalSha256 } from '@cakesplitter/shared-types';

export const STREAM_CHUNK_SIZE = 1024 * 1024;
export const DIRECT_FOLDER_MODE_ENABLED = false;

export interface StreamProgress {
  bytesRead: number;
  totalBytes: number;
}

export async function streamBlob(
  blob: Blob,
  onChunk: (chunk: Uint8Array) => void | Promise<void>,
  isCancelled: () => boolean = () => false,
  onProgress?: (progress: StreamProgress) => void,
): Promise<void> {
  const reader = blob.stream().getReader();
  let bytesRead = 0;
  try {
    for (;;) {
      if (isCancelled()) {
        throw new DOMException('Operation cancelled', 'AbortError');
      }
      const { done, value } = await reader.read();
      if (done) {
        break;
      }
      await onChunk(value);
      bytesRead += value.byteLength;
      onProgress?.({ bytesRead, totalBytes: blob.size });
    }
  } finally {
    reader.releaseLock();
  }
}

export async function hashBlob(
  blob: Blob,
  isCancelled?: () => boolean,
  onProgress?: (progress: StreamProgress) => void,
): Promise<string> {
  const hasher = new IncrementalSha256();
  await streamBlob(
    blob,
    (chunk) => {
      hasher.update(chunk);
    },
    isCancelled,
    onProgress,
  );
  return hasher.digestHex();
}

export function supportsDirectFolderMode(): boolean {
  return DIRECT_FOLDER_MODE_ENABLED;
}
