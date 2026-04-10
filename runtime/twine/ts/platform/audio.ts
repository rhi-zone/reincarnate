// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser audio — HTMLAudioElement wrapper. */

// HANDWRITTEN
export type AudioHandle = HTMLAudioElement;

// HANDWRITTEN
export function createAudio(sources: string[]): AudioHandle {
  const el = document.createElement("audio");
  for (const src of sources) {
    const source = document.createElement("source");
    source.src = src;
    // Infer type from extension
    const ext = src.split(".").pop()?.toLowerCase();
    if (ext === "mp3") source.type = "audio/mpeg";
    else if (ext === "ogg") source.type = "audio/ogg";
    else if (ext === "wav") source.type = "audio/wav";
    else if (ext === "m4a" || ext === "aac") source.type = "audio/mp4";
    else if (ext === "webm") source.type = "audio/webm";
    else if (ext === "flac") source.type = "audio/flac";
    el.appendChild(source);
  }
  el.preload = "auto";
  return el;
}

// HANDWRITTEN
export function playAudio(el: AudioHandle): Promise<void> {
  return el.play();
}

// HANDWRITTEN
export function pauseAudio(el: AudioHandle): void {
  el.pause();
}

// HANDWRITTEN
export function stopAudio(el: AudioHandle): void {
  el.pause();
  el.currentTime = 0;
}

// HANDWRITTEN
export function setVolume(el: AudioHandle, vol: number): void {
  el.volume = Math.max(0, Math.min(1, vol));
}

// HANDWRITTEN
export function setMuted(el: AudioHandle, muted: boolean): void {
  el.muted = muted;
}

// HANDWRITTEN
export function setLoop(el: AudioHandle, loop: boolean): void {
  el.loop = loop;
}

// HANDWRITTEN
export function seekAudio(el: AudioHandle, time: number): void {
  el.currentTime = time;
}

// HANDWRITTEN
export function getAudioDuration(el: AudioHandle): number {
  return el.duration;
}

// HANDWRITTEN
export function getAudioTime(el: AudioHandle): number {
  return el.currentTime;
}

// HANDWRITTEN
export function fadeAudio(
  el: AudioHandle,
  to: number,
  duration: number,
): Promise<void> {
  return new Promise((resolve) => {
    const from = el.volume;
    const steps = Math.max(1, Math.round(duration / 25));
    const delta = (to - from) / steps;
    let step = 0;
    const id = setInterval(() => {
      step++;
      if (step >= steps) {
        el.volume = Math.max(0, Math.min(1, to));
        clearInterval(id);
        resolve();
      } else {
        el.volume = Math.max(0, Math.min(1, from + delta * step));
      }
    }, 25);
  });
}

// HANDWRITTEN
export function isAudioReady(el: AudioHandle): boolean {
  return el.readyState >= HTMLMediaElement.HAVE_ENOUGH_DATA;
}
