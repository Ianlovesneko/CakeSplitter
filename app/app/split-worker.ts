const CHUNK_SIZE = 8 * 1024 * 1024;
const SHA256_K = new Uint32Array([
  0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
  0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
  0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
  0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
  0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
  0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
  0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
  0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
]);

function rotateRight(value: number, amount: number) {
  return (value >>> amount) | (value << (32 - amount));
}

class Sha256 {
  private state = new Uint32Array([0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19]);
  private buffer = new Uint8Array(64);
  private bufferLength = 0;
  private bytesHashed = 0n;

  update(input: Uint8Array) {
    this.bytesHashed += BigInt(input.byteLength);
    let offset = 0;
    if (this.bufferLength > 0) {
      const needed = 64 - this.bufferLength;
      const copied = Math.min(needed, input.byteLength);
      this.buffer.set(input.subarray(0, copied), this.bufferLength);
      this.bufferLength += copied;
      offset += copied;
      if (this.bufferLength === 64) {
        this.compress(this.buffer);
        this.bufferLength = 0;
      }
    }
    while (offset + 64 <= input.byteLength) {
      this.compress(input.subarray(offset, offset + 64));
      offset += 64;
    }
    if (offset < input.byteLength) {
      this.buffer.set(input.subarray(offset), 0);
      this.bufferLength = input.byteLength - offset;
    }
  }

  digest() {
    const bitLength = this.bytesHashed * 8n;
    this.buffer[this.bufferLength++] = 0x80;
    if (this.bufferLength > 56) {
      this.buffer.fill(0, this.bufferLength);
      this.compress(this.buffer);
      this.bufferLength = 0;
    }
    this.buffer.fill(0, this.bufferLength, 56);
    for (let index = 0; index < 8; index += 1) {
      this.buffer[63 - index] = Number((bitLength >> BigInt(index * 8)) & 0xffn);
    }
    this.compress(this.buffer);
    return Array.from(this.state, (word) => word.toString(16).padStart(8, "0")).join("");
  }

  private compress(block: Uint8Array) {
    const schedule = new Uint32Array(64);
    for (let index = 0; index < 16; index += 1) {
      const offset = index * 4;
      schedule[index] = ((block[offset] << 24) | (block[offset + 1] << 16) | (block[offset + 2] << 8) | block[offset + 3]) >>> 0;
    }
    for (let index = 16; index < 64; index += 1) {
      const a = schedule[index - 15];
      const b = schedule[index - 2];
      const sigma0 = rotateRight(a, 7) ^ rotateRight(a, 18) ^ (a >>> 3);
      const sigma1 = rotateRight(b, 17) ^ rotateRight(b, 19) ^ (b >>> 10);
      schedule[index] = (schedule[index - 16] + sigma0 + schedule[index - 7] + sigma1) >>> 0;
    }

    let [a, b, c, d, e, f, g, h] = this.state;
    for (let index = 0; index < 64; index += 1) {
      const sum1 = rotateRight(e, 6) ^ rotateRight(e, 11) ^ rotateRight(e, 25);
      const choice = (e & f) ^ (~e & g);
      const temp1 = (h + sum1 + choice + SHA256_K[index] + schedule[index]) >>> 0;
      const sum0 = rotateRight(a, 2) ^ rotateRight(a, 13) ^ rotateRight(a, 22);
      const majority = (a & b) ^ (a & c) ^ (b & c);
      const temp2 = (sum0 + majority) >>> 0;
      h = g;
      g = f;
      f = e;
      e = (d + temp1) >>> 0;
      d = c;
      c = b;
      b = a;
      a = (temp1 + temp2) >>> 0;
    }
    this.state[0] = (this.state[0] + a) >>> 0;
    this.state[1] = (this.state[1] + b) >>> 0;
    this.state[2] = (this.state[2] + c) >>> 0;
    this.state[3] = (this.state[3] + d) >>> 0;
    this.state[4] = (this.state[4] + e) >>> 0;
    this.state[5] = (this.state[5] + f) >>> 0;
    this.state[6] = (this.state[6] + g) >>> 0;
    this.state[7] = (this.state[7] + h) >>> 0;
  }
}

type SliceRecord = { index: number; name: string; size: number; sha256: string };
type Manifest = { originalSha256: string; slices: SliceRecord[] };
const workerScope = globalThis as unknown as { postMessage(message: unknown): void; onmessage: ((event: MessageEvent) => void) | null };

async function hashBlob(blob: Blob, onChunk: (processed: number) => void) {
  const hasher = new Sha256();
  for (let offset = 0; offset < blob.size; offset += CHUNK_SIZE) {
    const chunk = new Uint8Array(await blob.slice(offset, Math.min(offset + CHUNK_SIZE, blob.size)).arrayBuffer());
    hasher.update(chunk);
    onChunk(chunk.byteLength);
  }
  return hasher.digest();
}

function progress(phase: string, processedBytes: number, totalBytes: number, currentIndex: number, totalSlices: number) {
  workerScope.postMessage({ type: "progress", phase, processedBytes, totalBytes, currentIndex, totalSlices });
}

async function splitFile(file: File, sliceSizeBytes: number) {
  const totalSlices = Math.max(1, Math.ceil(file.size / sliceSizeBytes));
  const records: SliceRecord[] = [];
  const wholeHasher = new Sha256();
  let processedBytes = 0;
  for (let index = 0; index < totalSlices; index += 1) {
    const start = index * sliceSizeBytes;
    const end = Math.min(file.size, start + sliceSizeBytes);
    const blob = file.slice(start, end);
    const sliceHasher = new Sha256();
    for (let offset = 0; offset < blob.size; offset += CHUNK_SIZE) {
      const chunk = new Uint8Array(await blob.slice(offset, Math.min(offset + CHUNK_SIZE, blob.size)).arrayBuffer());
      sliceHasher.update(chunk);
      wholeHasher.update(chunk);
      processedBytes += chunk.byteLength;
      progress("hashing", processedBytes, file.size, index, totalSlices);
    }
    records.push({ index, name: `${file.name}.${String(index + 1).padStart(3, "0")}.slice`, size: blob.size, sha256: sliceHasher.digest() });
    workerScope.postMessage({ type: "slice", record: records[index] });
  }
  const manifest = {
    formatVersion: "1.0",
    originalName: file.name,
    originalSize: file.size,
    sliceSizeBytes,
    originalSha256: wholeHasher.digest(),
    slices: records,
  };
  workerScope.postMessage({ type: "complete", manifest });
}

async function rebuildFiles(manifest: Manifest, slices: Array<{ record: SliceRecord; blob: Blob }>) {
  const ordered = [...slices].sort((a, b) => a.record.index - b.record.index);
  if (ordered.length !== manifest.slices.length) throw new Error("The manifest and selected Slice count do not match.");
  const rebuiltParts: Blob[] = [];
  let processedBytes = 0;
  const totalBytes = ordered.reduce((sum, item) => sum + item.blob.size, 0);
  for (let index = 0; index < ordered.length; index += 1) {
    const item = ordered[index];
    const expected = manifest.slices[index];
    if (!expected || item.record.name !== expected.name || item.blob.size !== expected.size) throw new Error(`Slice ${index + 1} does not match the manifest.`);
    const actualHash = await hashBlob(item.blob, (chunkSize) => {
      processedBytes += chunkSize;
      progress("verifying", processedBytes, totalBytes, index, ordered.length);
    });
    if (actualHash !== expected.sha256) throw new Error(`SHA-256 verification failed for ${expected.name}.`);
    rebuiltParts.push(item.blob);
  }
  const rebuiltBlob = new Blob(rebuiltParts);
  const rebuiltHash = await hashBlob(rebuiltBlob, (chunkSize) => {
    processedBytes += chunkSize;
    progress("rebuilding", processedBytes, totalBytes * 2, ordered.length, ordered.length);
  });
  if (rebuiltHash !== manifest.originalSha256) throw new Error("Rebuilt file SHA-256 does not match the manifest.");
  workerScope.postMessage({ type: "complete", rebuiltHash, size: rebuiltBlob.size });
}

workerScope.onmessage = async (event: MessageEvent) => {
  try {
    if (event.data?.type === "split") await splitFile(event.data.file, event.data.sliceSizeBytes);
    if (event.data?.type === "rebuild") await rebuildFiles(event.data.manifest, event.data.slices);
  } catch (error) {
    workerScope.postMessage({ type: "error", message: error instanceof Error ? error.message : "The local operation failed." });
  }
};
