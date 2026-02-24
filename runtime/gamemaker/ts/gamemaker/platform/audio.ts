/** Web Audio API — platform audio implementation for GML sound functions. */

interface PlayingSound {
  source: AudioBufferSourceNode;
  gainNode: GainNode;
  soundIndex: number;
  loop: boolean;
  pitch: number;
  /** audioCtx.currentTime at which playback started (adjusted for offset). */
  startTime: number;
  paused: boolean;
  /** Playback position when paused, in seconds. */
  pauseOffset: number;
}

export class AudioState {
  ctx: AudioContext | null = null;
  masterGainNode: GainNode | null = null;
  /** Decoded buffers indexed by sound index (SOND order). */
  buffers: (AudioBuffer | null)[] = [];
  playing = new Map<number, PlayingSound>();
  nextHandle = 1;
}

export async function loadAudio(
  state: AudioState,
  sounds: { name: string; url: string }[],
): Promise<void> {
  state.ctx = new AudioContext();
  state.masterGainNode = state.ctx.createGain();
  state.masterGainNode.connect(state.ctx.destination);

  const promises = sounds.map(async (s, i) => {
    if (!s.url) { state.buffers[i] = null; return; }
    try {
      const res = await fetch(s.url);
      if (!res.ok) { state.buffers[i] = null; return; }
      state.buffers[i] = await state.ctx!.decodeAudioData(await res.arrayBuffer());
    } catch {
      state.buffers[i] = null;
    }
  });
  await Promise.all(promises);
}

function _makeSource(
  state: AudioState,
  handle: number,
  buffer: AudioBuffer,
  gainNode: GainNode,
  loop: boolean,
  pitch: number,
  offset: number,
): AudioBufferSourceNode {
  const source = state.ctx!.createBufferSource();
  source.buffer = buffer;
  source.loop = loop;
  source.playbackRate.value = pitch;
  source.connect(gainNode);
  source.start(0, offset);
  source.onended = () => {
    const p = state.playing.get(handle);
    if (p && !p.paused) state.playing.delete(handle);
  };
  return source;
}

export function audioPlay(
  state: AudioState,
  soundIndex: number,
  loop: boolean,
  gain = 1,
  offset = 0,
  pitch = 1,
): number {
  if (!state.ctx || !state.masterGainNode) return -1;
  const buffer = state.buffers[soundIndex];
  if (!buffer) return -1;
  if (state.ctx.state === "suspended") void state.ctx.resume();

  const gainNode = state.ctx.createGain();
  gainNode.gain.value = gain;
  gainNode.connect(state.masterGainNode);

  const handle = state.nextHandle++;
  const source = _makeSource(state, handle, buffer, gainNode, loop, pitch, offset);

  state.playing.set(handle, {
    source, gainNode, soundIndex, loop, pitch,
    startTime: state.ctx.currentTime - offset,
    paused: false, pauseOffset: 0,
  });
  return handle;
}

export function audioStop(state: AudioState, handle: number): void {
  const p = state.playing.get(handle);
  if (!p) return;
  try { p.source.stop(); } catch { /* already ended */ }
  state.playing.delete(handle);
}

export function audioStopAll(state: AudioState): void {
  for (const handle of [...state.playing.keys()]) audioStop(state, handle);
}

export function audioPause(state: AudioState, handle: number): void {
  const p = state.playing.get(handle);
  if (!p || !state.ctx || p.paused) return;
  p.pauseOffset = state.ctx.currentTime - p.startTime;
  p.paused = true;
  try { p.source.stop(); } catch { /* already ended */ }
}

export function audioResume(state: AudioState, handle: number): void {
  const p = state.playing.get(handle);
  if (!p || !state.ctx || !p.paused) return;
  const buffer = state.buffers[p.soundIndex];
  if (!buffer) return;
  p.source = _makeSource(state, handle, buffer, p.gainNode, p.loop, p.pitch, p.pauseOffset);
  p.startTime = state.ctx.currentTime - p.pauseOffset;
  p.paused = false;
}

export function audioResumeAll(state: AudioState): void {
  for (const handle of state.playing.keys()) audioResume(state, handle);
}

export function audioIsPlaying(state: AudioState, handle: number): boolean {
  const p = state.playing.get(handle);
  return p !== undefined && !p.paused;
}

export function audioSetGain(state: AudioState, handle: number, gain: number, timeMs: number): void {
  const p = state.playing.get(handle);
  if (!p || !state.ctx) return;
  p.gainNode.gain.linearRampToValueAtTime(gain, state.ctx.currentTime + timeMs / 1000);
}

export function audioGetGain(state: AudioState, handle: number): number {
  return state.playing.get(handle)?.gainNode.gain.value ?? 0;
}

export function audioSetPitch(state: AudioState, handle: number, pitch: number): void {
  const p = state.playing.get(handle);
  if (!p) return;
  p.source.playbackRate.value = pitch;
  p.pitch = pitch;
}

export function audioGetPitch(state: AudioState, handle: number): number {
  return state.playing.get(handle)?.pitch ?? 1;
}

export function audioSetMasterGain(state: AudioState, gain: number): void {
  if (state.masterGainNode) state.masterGainNode.gain.value = gain;
}

export function audioGetTrackPosition(state: AudioState, handle: number): number {
  const p = state.playing.get(handle);
  if (!p || !state.ctx) return 0;
  return p.paused ? p.pauseOffset : state.ctx.currentTime - p.startTime;
}

export function audioSetTrackPosition(state: AudioState, handle: number, pos: number): void {
  const p = state.playing.get(handle);
  if (!p) return;
  // AudioBufferSourceNode cannot seek — restart at new position.
  const loop = p.source.loop;
  const gain = p.gainNode.gain.value;
  const pitch = p.pitch;
  const si = p.soundIndex;
  audioStop(state, handle);
  // Re-insert at same handle so callers' handles remain valid.
  if (!state.ctx || !state.masterGainNode) return;
  const buffer = state.buffers[si];
  if (!buffer) return;
  if (state.ctx.state === "suspended") void state.ctx.resume();
  const gainNode = state.ctx.createGain();
  gainNode.gain.value = gain;
  gainNode.connect(state.masterGainNode);
  const source = _makeSource(state, handle, buffer, gainNode, loop, pitch, pos);
  state.playing.set(handle, {
    source, gainNode, soundIndex: si, loop, pitch,
    startTime: state.ctx.currentTime - pos,
    paused: false, pauseOffset: 0,
  });
}

export function audioSoundLength(state: AudioState, soundIndex: number): number {
  return state.buffers[soundIndex]?.duration ?? 0;
}
