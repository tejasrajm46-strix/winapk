// Android screenrecord emits Annex-B H.264. Keep at most one incomplete NAL and
// one access unit; do not invent frame boundaries from arbitrary IPC chunks.
export class PhoneVideo {
  private pending = new Uint8Array(0);
  private parts: Uint8Array[] = [];
  private hasSlice = false;
  private key = false;
  private decoder: VideoDecoder;
  private timestamp = 0;
  private waitingForKey = true;
  private stopped = false;
  private idleTimer: number | undefined;

  constructor(
    private readonly canvas: HTMLCanvasElement,
    private readonly onFrame: (width: number, height: number) => void,
    private readonly onError: (error: Error) => void,
  ) {
    this.decoder = new VideoDecoder({
      output: (frame) => {
        try {
          if (this.stopped) return;
          if (this.canvas.width !== frame.displayWidth || this.canvas.height !== frame.displayHeight) {
            this.canvas.width = frame.displayWidth;
            this.canvas.height = frame.displayHeight;
          }
          this.canvas.getContext("2d")?.drawImage(frame, 0, 0);
          this.onFrame(frame.displayWidth, frame.displayHeight);
        } finally {
          frame.close();
        }
      },
      error: (error) => this.onError(error),
    });
  }

  push(data: ArrayBuffer) {
    if (this.stopped) return;
    const bytes = new Uint8Array(data);
    if (!bytes.length) throw new Error("Android video capture ended unexpectedly.");
    const combined = new Uint8Array(this.pending.length + bytes.length);
    combined.set(this.pending);
    combined.set(bytes, this.pending.length);
    if (combined.length > 8 * 1024 * 1024) throw new Error("Android video frame exceeded the capture limit.");
    const starts: { at: number; length: number }[] = [];
    for (let i = 0; i + 3 < combined.length; i++) {
      if (combined[i] !== 0 || combined[i + 1] !== 0) continue;
      if (combined[i + 2] === 1) { starts.push({ at: i, length: 3 }); i += 2; }
      else if (combined[i + 2] === 0 && combined[i + 3] === 1) { starts.push({ at: i, length: 4 }); i += 3; }
    }
    for (let i = 0; i + 1 < starts.length; i++) {
      const start = starts[i];
      this.nal(combined.slice(start.at, starts[i + 1].at), start.length);
    }
    this.pending = starts.length ? combined.slice(starts[starts.length - 1].at) : combined;
    // As soon as the next first slice begins, the previous access unit is
    // complete; waiting for the whole next slice would stall a static screen.
    if (starts.length && this.hasSlice) {
      const last = starts[starts.length - 1];
      const type = this.pending[last.length] & 31;
      if ((type === 1 || type === 5) && (this.pending[last.length + 1] & 0x80)) this.flush();
    }
    // screenrecord omits a terminator after its last frame on a static screen.
    // A short quiet period makes that frame visible without waiting for input.
    window.clearTimeout(this.idleTimer);
    this.idleTimer = window.setTimeout(() => {
      try {
        const prefix = this.pending[2] === 1 ? 3 : this.pending[3] === 1 ? 4 : 0;
        if (prefix && this.pending.length > prefix) {
          this.nal(this.pending, prefix);
          this.pending = new Uint8Array(0);
          this.flush();
        }
      } catch (error) { this.onError(error instanceof Error ? error : new Error(String(error))); }
    }, 80);
  }

  private nal(bytes: Uint8Array, prefix: number) {
    const type = bytes[prefix] & 31;
    if (type === 7) {
      if (bytes.length < prefix + 4) throw new Error("Invalid H.264 configuration.");
      const codec = `avc1.${Array.from(bytes.subarray(prefix + 1, prefix + 4), (b) => b.toString(16).padStart(2, "0")).join("")}`;
      if (this.decoder.state === "unconfigured") this.decoder.configure({ codec, optimizeForLatency: true });
    }
    const isSlice = type === 1 || type === 5;
    // first_mb_in_slice is an unsigned Exp-Golomb field: its zero value starts
    // with a 1 bit. A new first slice finishes the previous access unit.
    const firstSlice = isSlice && !!(bytes[prefix + 1] & 0x80);
    if (this.hasSlice && (firstSlice || type === 7 || type === 9)) this.flush();
    this.parts.push(bytes);
    if (isSlice) { this.hasSlice = true; this.key ||= type === 5; }
  }

  private flush() {
    if (!this.hasSlice) return;
    if (this.decoder.state !== "configured") throw new Error("Android video has no decoder configuration.");
    if (this.decoder.decodeQueueSize > 30) throw new Error("Phone video decoder fell behind.");
    const size = this.parts.reduce((sum, part) => sum + part.length, 0);
    const frame = new Uint8Array(size);
    let offset = 0;
    for (const part of this.parts) { frame.set(part, offset); offset += part.length; }
    if (!this.waitingForKey || this.key) {
      this.timestamp = Math.max(this.timestamp + 1, Math.floor(performance.now() * 1000));
      this.decoder.decode(new EncodedVideoChunk({ type: this.key ? "key" : "delta", timestamp: this.timestamp, data: frame }));
      this.waitingForKey = false;
    }
    this.parts = [];
    this.hasSlice = false;
    this.key = false;
  }

  close() {
    this.stopped = true;
    window.clearTimeout(this.idleTimer);
    if (this.decoder.state !== "closed") this.decoder.close();
    this.parts = [];
    this.pending = new Uint8Array(0);
  }
}
