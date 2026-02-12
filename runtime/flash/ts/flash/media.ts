/**
 * flash.media package — SoundTransform, Sound, SoundChannel, SoundMixer,
 * SoundLoaderContext, ID3Info, Microphone, Camera, Video.
 */

import { EventDispatcher, Event } from "./events";

// ---------------------------------------------------------------------------
// SoundTransform
// ---------------------------------------------------------------------------

export class SoundTransform {
  volume: number;
  pan: number;
  leftToLeft: number;
  leftToRight: number;
  rightToLeft: number;
  rightToRight: number;

  constructor(vol = 1, panning = 0) {
    this.volume = vol;
    this.pan = panning;
    this.leftToLeft = 1;
    this.leftToRight = 0;
    this.rightToLeft = 0;
    this.rightToRight = 1;
  }
}

// ---------------------------------------------------------------------------
// SoundLoaderContext
// ---------------------------------------------------------------------------

export class SoundLoaderContext {
  bufferTime: number;
  checkPolicyFile: boolean;

  constructor(bufferTime = 1000, checkPolicyFile = false) {
    this.bufferTime = bufferTime;
    this.checkPolicyFile = checkPolicyFile;
  }
}

// ---------------------------------------------------------------------------
// ID3Info
// ---------------------------------------------------------------------------

export class ID3Info {
  album: string | null = null;
  artist: string | null = null;
  comment: string | null = null;
  genre: string | null = null;
  songName: string | null = null;
  track: string | null = null;
  year: string | null = null;
}

// ---------------------------------------------------------------------------
// SoundChannel
// ---------------------------------------------------------------------------

export class SoundChannel extends EventDispatcher {
  leftPeak = 0;
  rightPeak = 0;
  position = 0;
  soundTransform: SoundTransform = new SoundTransform();

  stop(): void {
    this.dispatchEvent(new Event(Event.SOUND_COMPLETE));
  }
}

// ---------------------------------------------------------------------------
// Sound
// ---------------------------------------------------------------------------

export class Sound extends EventDispatcher {
  bytesLoaded = 0;
  bytesTotal = 0;
  id3: ID3Info = new ID3Info();
  isBuffering = false;
  isURLInaccessible = false;
  length = 0;
  url = "";

  close(): void {}

  play(
    startTime = 0,
    _loops = 0,
    _sndTransform: SoundTransform | null = null,
  ): SoundChannel {
    void startTime;
    return new SoundChannel();
  }

  load(_stream: any, _context?: SoundLoaderContext): void {}

  extract(_target: any, _length: number, _startPosition = -1): number {
    return 0;
  }
}

// ---------------------------------------------------------------------------
// SoundMixer
// ---------------------------------------------------------------------------

export class SoundMixer {
  static bufferTime = 0;
  static soundTransform: SoundTransform = new SoundTransform();

  static areSoundsInaccessible(): boolean {
    return false;
  }

  static stopAll(): void {}

  static computeSpectrum(
    _outputArray: any,
    _FFTMode = false,
    _stretchFactor = 0,
  ): void {}
}

// ---------------------------------------------------------------------------
// Microphone (stub — no browser capture yet)
// ---------------------------------------------------------------------------

export class Microphone extends EventDispatcher {
  activityLevel = -1;
  gain = 50;
  index = -1;
  muted = true;
  name = "";
  rate = 8;
  silenceLevel = 10;
  silenceTimeout = 2000;
  soundTransform: SoundTransform = new SoundTransform();
  useEchoSuppression = false;

  static readonly names: string[] = [];

  static getMicrophone(_index = -1): Microphone | null {
    return null;
  }

  setLoopBack(_state = true): void {}
  setSilenceLevel(_silenceLevel: number, _timeout = 2000): void {}
  setUseEchoSuppression(_useEchoSuppression: boolean): void {}
}

// ---------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------

export class Video extends EventDispatcher {
  deblocking = 0;
  smoothing = false;
  videoWidth = 0;
  videoHeight = 0;

  constructor(public width = 320, public height = 240) {
    super();
  }

  attachNetStream(_netStream: any): void {}
  clear(): void {}
}
