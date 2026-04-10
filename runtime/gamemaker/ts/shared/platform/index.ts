// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

export type { CanvasHandle, FontHandle, PathHandle, GradientHandle, BlendMode, TextAlign, TextBaseline, LineCap, LineJoin } from "./graphics";
export { GraphicsState, initSurface, createCanvas, resizeCanvas, canvasWidth, canvasHeight, readCanvasPixels, canvasToImage, createImageBitmapAsync, loadFont, createPath, pathMoveTo, pathLineTo, pathBezierTo, pathQuadraticTo, pathArc, pathClose, destroyPath, destroyCanvas, destroyFont, createLinearGradient, createRadialGradient, gradientAddStop, destroyGradient, setTransform, setAlpha, setBlendMode, setColorTransform, setImageSmoothing, setStrokeStyle, setDashPattern, saveState, restoreState, resetCanvasState, clearCanvas, fillRect, drawImage, drawCanvas, drawText, measureText, beginPath, moveTo, lineTo, bezierTo, quadraticTo, arc, closePath, fillPath, fillPathGradient, strokePath, clip, beginTextPath, fillPathHandle, fillPathHandleGradient, strokePathHandle, clipPathHandle } from "./graphics";
// Legacy exports preserved for backward compatibility:
export { GraphicsContext, initCanvas, initWebGL } from "./graphics";
export type { ImageHandle } from "./images";
export { ImageState, createImage, loadImageUrl, loadImageBytes, createSubImage, imageWidth, imageHeight, readPixels, writePixels, destroyImage } from "./images";
export type { DeviceKind } from "./input";
export { InputState, devices, onDeviceConnect, onDeviceDisconnect, onKeyDown, onKeyUp, isKeyDown, onMouseDown, onMouseUp, onMouseMove, onScroll, isMouseDown, mouseX, mouseY, requestPointerLock, releasePointerLock, isPointerLocked, onMouseDelta, onTouchStart, onTouchMove, onTouchEnd, touchCount, touchX, touchY, deviceAxis, deviceButtonPressed, deviceButtonValue, deviceConnected, deviceDescription, deviceCount, snapshotGamepadButtons, gamepadSlotCount, onTextInput, onCompositionStart, onCompositionUpdate, onCompositionEnd } from "./input";
export type { DelayHandle, RecurringHandle, FrameHandle } from "./timing";
export { scheduleDelayed, cancelDelayed, scheduleRecurring, cancelRecurring, requestFrame, cancelFrame, currentTimeMs, currentWallTimeMs } from "./timing";
export { PersistenceState, initPersistence, storePersistence, fetchPersistence, removePersistence, listPersistenceKeys } from "./persistence";
export { systemLanguage, isNetworkConnected, writeClipboard } from "./system";
export { displayWidth, displayHeight, openUrl, closeWindow, requestFullscreen, exitFullscreen, downloadDataUrl, setWindowTitle, windowHasFocus } from "./window";
export type { NodeKind, ParamKind, BufferHandle, NodeHandle, VoiceHandle, GroupHandle } from "./audio";
export {
  AudioState,
  loadAudio, audioReady,
  createNode, connect, disconnect, setNodeParam, getNodeParam, destroyNode,
  play, stop, stopAll, pause, resume, resumeAll,
  isPlaying, isPaused,
  onVoiceEnd,
  setVoiceGain, getVoiceGain,
  setVoicePitch, getVoicePitch,
  setVoicePan, getVoicePan,
  setMasterGain,
  getPosition, setPosition, bufferDuration, destroyBuffer,
  stopNode, pauseNode, resumeNode,
  createVoiceGroup, addToGroup, removeFromGroup,
  stopGroup, pauseGroup, resumeGroup, setGroupGain, destroyGroup,
} from "./audio";
