/** Browser audio — HTMLAudioElement wrapper. */

export type AudioHandle = HTMLAudioElement;

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

export function playAudio(el: AudioHandle): Promise<void> {
  return el.play();
}

export function pauseAudio(el: AudioHandle): void {
  el.pause();
}

export function stopAudio(el: AudioHandle): void {
  el.pause();
  el.currentTime = 0;
}

export function setVolume(el: AudioHandle, vol: number): void {
  el.volume = Math.max(0, Math.min(1, vol));
}

export function setMuted(el: AudioHandle, muted: boolean): void {
  el.muted = muted;
}

export function setLoop(el: AudioHandle, loop: boolean): void {
  el.loop = loop;
}

export function seekAudio(el: AudioHandle, time: number): void {
  el.currentTime = time;
}

export function getAudioDuration(el: AudioHandle): number {
  return el.duration;
}

export function getAudioTime(el: AudioHandle): number {
  return el.currentTime;
}

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

export function isAudioReady(el: AudioHandle): boolean {
  return el.readyState >= HTMLMediaElement.HAVE_ENOUGH_DATA;
}
