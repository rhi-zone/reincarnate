/** GML audio functions — backed by the Web Audio API platform module. */

import type { GameRuntime } from "./runtime";
import {
  AudioState, loadAudio,
  audioPlay, audioStop, audioStopAll, audioPause, audioResume, audioResumeAll,
  audioIsPlaying, audioSetGain, audioGetGain, audioSetPitch, audioGetPitch,
  audioSetMasterGain, audioGetTrackPosition, audioSetTrackPosition, audioSoundLength,
} from "./platform/audio";

export { AudioState, loadAudio };

export function createAudioAPI(rt: GameRuntime) {
  const audio = rt._audio;

  function audio_play_sound(
    sound: number, _priority: number, loop: boolean,
    gain = 1, offset = 0, pitch = 1,
  ): number {
    return audioPlay(audio, sound, loop, gain, offset, pitch);
  }

  function audio_play_sound_at(
    sound: number, _x: number, _y: number, _z: number,
    _falloff: number, _min: number, _max: number,
    _priority: number, loop: boolean,
  ): number {
    // Positional audio: play without spatial attenuation (no Web Audio PannerNode yet).
    return audioPlay(audio, sound, loop);
  }

  function audio_stop_sound(handle: number): void { audioStop(audio, handle); }
  function audio_stop_all(): void { audioStopAll(audio); }
  function audio_pause_sound(handle: number): void { audioPause(audio, handle); }
  function audio_resume_sound(handle: number): void { audioResume(audio, handle); }
  function audio_resume_all(): void { audioResumeAll(audio); }
  function audio_is_playing(handle: number): boolean { return audioIsPlaying(audio, handle); }

  function audio_sound_gain(handle: number, gain: number, timeMs: number): void {
    audioSetGain(audio, handle, gain, timeMs);
  }
  function audio_sound_get_gain(handle: number): number { return audioGetGain(audio, handle); }
  function audio_sound_pitch(handle: number, pitch: number): void { audioSetPitch(audio, handle, pitch); }
  function audio_sound_get_pitch(handle: number): number { return audioGetPitch(audio, handle); }
  function audio_master_gain(gain: number): void { audioSetMasterGain(audio, gain); }

  function audio_sound_set_track_position(handle: number, pos: number): void {
    audioSetTrackPosition(audio, handle, pos);
  }
  function audio_sound_get_track_position(handle: number): number {
    return audioGetTrackPosition(audio, handle);
  }
  function audio_sound_length(sound: number): number { return audioSoundLength(audio, sound); }

  function audio_exists(sound: number): boolean {
    return sound >= 0 && sound < rt.sounds.length && rt.sounds[sound]?.url !== "";
  }
  function audio_get_name(sound: number): string { return rt.sounds[sound]?.name ?? ""; }

  // Audio groups: in GML these batch sounds for volume control.
  // Without group metadata we treat group ops as master-gain adjustments.
  function audio_group_load(_group: number): void { /* no-op — all audio loaded at startup */ }
  function audio_group_stop_all(_group: number): void { audioStopAll(audio); }
  function audio_group_set_gain(_group: number, gain: number, timeMs: number): void {
    audioSetMasterGain(audio, gain);
    void timeMs; // ramp not implemented for master gain
  }

  return {
    audio_play_sound, audio_play_sound_at,
    audio_stop_sound, audio_stop_all,
    audio_pause_sound, audio_resume_sound, audio_resume_all,
    audio_is_playing,
    audio_sound_gain, audio_sound_get_gain,
    audio_sound_pitch, audio_sound_get_pitch,
    audio_master_gain,
    audio_sound_set_track_position, audio_sound_get_track_position,
    audio_sound_length,
    audio_exists, audio_get_name,
    audio_group_load, audio_group_stop_all, audio_group_set_gain,
  };
}
