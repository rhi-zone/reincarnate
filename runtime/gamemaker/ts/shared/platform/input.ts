// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser input — keyboard, mouse, touch, gamepad, and text input event binding. */

// HANDWRITTEN
export type DeviceKind = "keyboard" | "mouse" | "touch" | "gamepad";

// HANDWRITTEN
export class InputState {
  keysDown = new Set<string>();
  mouseButtons = new Set<number>();
  mouseX = 0;
  mouseY = 0;
  pointerLocked = false;
  touches = new Map<number, { x: number; y: number }>();
  gamepads = new Map<number, Gamepad>();
}

// --- Device enumeration ---

// HANDWRITTEN
export function devices(state: InputState, kind: DeviceKind): number[] {
  switch (kind) {
    case "keyboard":
    case "mouse":
      return [0];
    case "touch":
      return [0];
    case "gamepad": {
      const pads = navigator.getGamepads();
      const indices: number[] = [];
      for (let i = 0; i < pads.length; i++) {
        if (pads[i] != null) indices.push(i);
      }
      return indices;
    }
  }
}

// HANDWRITTEN
export function onDeviceConnect(
  _state: InputState,
  cb: (device: number, kind: DeviceKind) => void,
): void {
  window.addEventListener("gamepadconnected", (e) => {
    cb((e as GamepadEvent).gamepad.index, "gamepad");
  });
}

// HANDWRITTEN
export function onDeviceDisconnect(
  _state: InputState,
  cb: (device: number) => void,
): void {
  window.addEventListener("gamepaddisconnected", (e) => {
    cb((e as GamepadEvent).gamepad.index);
  });
}

// --- Keyboard ---

// HANDWRITTEN
export function onKeyDown(
  state: InputState,
  _canvas: HTMLCanvasElement,
  cb: (device: number, code: string, key: string) => void,
): void {
  window.addEventListener("keydown", (e) => {
    state.keysDown.add(e.code);
    cb(0, e.code, e.key);
  });
}

// HANDWRITTEN
export function onKeyUp(
  state: InputState,
  _canvas: HTMLCanvasElement,
  cb: (device: number, code: string, key: string) => void,
): void {
  window.addEventListener("keyup", (e) => {
    state.keysDown.delete(e.code);
    cb(0, e.code, e.key);
  });
}

// HANDWRITTEN
export function isKeyDown(state: InputState, _device: number, code: string): boolean {
  return state.keysDown.has(code);
}

// --- Mouse ---

// HANDWRITTEN
export function onMouseDown(
  state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, button: number) => void,
): void {
  canvas.addEventListener("mousedown", (e) => {
    state.mouseButtons.add(e.button);
    cb(0, e.button);
  });
  canvas.addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
  });
}

// HANDWRITTEN
export function onMouseUp(
  state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, button: number) => void,
): void {
  canvas.addEventListener("mouseup", (e) => {
    state.mouseButtons.delete(e.button);
    cb(0, e.button);
  });
}

// HANDWRITTEN
export function onMouseMove(
  state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, x: number, y: number) => void,
): void {
  canvas.addEventListener("mousemove", (e) => {
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    state.mouseX = x;
    state.mouseY = y;
    cb(0, x, y);
  });
}

// HANDWRITTEN
export function onScroll(
  _state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, dx: number, dy: number) => void,
): void {
  canvas.addEventListener(
    "wheel",
    (e) => {
      e.preventDefault();
      cb(0, e.deltaX, e.deltaY);
    },
    { passive: false },
  );
}

// HANDWRITTEN
export function isMouseDown(state: InputState, _device: number, button: number): boolean {
  return state.mouseButtons.has(button);
}

// HANDWRITTEN
export function mouseX(state: InputState, _device: number): number {
  return state.mouseX;
}

// HANDWRITTEN
export function mouseY(state: InputState, _device: number): number {
  return state.mouseY;
}

// --- Pointer lock ---

// HANDWRITTEN
export function requestPointerLock(canvas: HTMLCanvasElement): void {
  canvas.requestPointerLock();
}

// HANDWRITTEN
export function releasePointerLock(): void {
  document.exitPointerLock();
}

// HANDWRITTEN
export function isPointerLocked(state: InputState): boolean {
  return state.pointerLocked;
}

// HANDWRITTEN
export function onMouseDelta(
  _state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, dx: number, dy: number) => void,
): void {
  canvas.addEventListener("mousemove", (e) => {
    cb(0, e.movementX, e.movementY);
  });
}

// --- Touch ---

function touchPos(
  touch: Touch,
  canvas: HTMLCanvasElement,
): { x: number; y: number } {
  const rect = canvas.getBoundingClientRect();
  return { x: touch.clientX - rect.left, y: touch.clientY - rect.top };
}

// HANDWRITTEN
export function onTouchStart(
  state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, id: number, x: number, y: number) => void,
): void {
  canvas.addEventListener("touchstart", (e) => {
    e.preventDefault();
    for (const touch of Array.from(e.changedTouches)) {
      const pos = touchPos(touch, canvas);
      state.touches.set(touch.identifier, pos);
      cb(0, touch.identifier, pos.x, pos.y);
    }
  }, { passive: false });
}

// HANDWRITTEN
export function onTouchMove(
  state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, id: number, x: number, y: number) => void,
): void {
  canvas.addEventListener("touchmove", (e) => {
    e.preventDefault();
    for (const touch of Array.from(e.changedTouches)) {
      const pos = touchPos(touch, canvas);
      state.touches.set(touch.identifier, pos);
      cb(0, touch.identifier, pos.x, pos.y);
    }
  }, { passive: false });
}

// HANDWRITTEN
export function onTouchEnd(
  state: InputState,
  canvas: HTMLCanvasElement,
  cb: (device: number, id: number, x: number, y: number) => void,
): void {
  canvas.addEventListener("touchend", (e) => {
    for (const touch of Array.from(e.changedTouches)) {
      const pos = touchPos(touch, canvas);
      state.touches.delete(touch.identifier);
      cb(0, touch.identifier, pos.x, pos.y);
    }
  });
}

// HANDWRITTEN
export function touchCount(state: InputState, _device: number): number {
  return state.touches.size;
}

// HANDWRITTEN
export function touchX(state: InputState, _device: number, id: number): number {
  return state.touches.get(id)?.x ?? 0;
}

// HANDWRITTEN
export function touchY(state: InputState, _device: number, id: number): number {
  return state.touches.get(id)?.y ?? 0;
}

// --- Gamepad ---

// HANDWRITTEN
export function deviceAxis(
  _state: InputState,
  device: number,
  axis: number,
): number {
  return navigator.getGamepads()[device]?.axes[axis] ?? 0;
}

// HANDWRITTEN
export function deviceButtonPressed(
  _state: InputState,
  device: number,
  button: number,
): boolean {
  return navigator.getGamepads()[device]?.buttons[button]?.pressed ?? false;
}

// HANDWRITTEN
export function deviceButtonValue(
  _state: InputState,
  device: number,
  button: number,
): number {
  return navigator.getGamepads()[device]?.buttons[button]?.value ?? 0;
}

// HANDWRITTEN
export function deviceConnected(
  _state: InputState,
  device: number,
): boolean {
  return navigator.getGamepads()[device]?.connected ?? false;
}

// HANDWRITTEN
export function deviceDescription(
  _state: InputState,
  device: number,
): string {
  return navigator.getGamepads()[device]?.id ?? "";
}

// HANDWRITTEN
export function deviceCount(_state: InputState): number {
  return navigator.getGamepads().filter(g => g != null).length;
}

/** Snapshot the pressed state of all buttons for a gamepad (for pressed/released edge detection). */
// HANDWRITTEN
export function snapshotGamepadButtons(_state: InputState, device: number): boolean[] {
  const gp = navigator.getGamepads()[device];
  return gp ? gp.buttons.map(b => b.pressed) : [];
}

/** Return the number of gamepad slots (including disconnected). */
// HANDWRITTEN
export function gamepadSlotCount(_state: InputState): number {
  return navigator.getGamepads().length;
}

// --- Text input ---

// HANDWRITTEN
export function onTextInput(
  _state: InputState,
  _canvas: HTMLCanvasElement,
  cb: (device: number, text: string) => void,
): void {
  window.addEventListener("input", (e) => {
    cb(0, (e as InputEvent).data ?? "");
  });
}

// HANDWRITTEN
export function onCompositionStart(
  _state: InputState,
  _canvas: HTMLCanvasElement,
  cb: (device: number) => void,
): void {
  window.addEventListener("compositionstart", () => {
    cb(0);
  });
}

// HANDWRITTEN
export function onCompositionUpdate(
  _state: InputState,
  _canvas: HTMLCanvasElement,
  cb: (device: number, text: string) => void,
): void {
  window.addEventListener("compositionupdate", (e) => {
    cb(0, (e as CompositionEvent).data);
  });
}

// HANDWRITTEN
export function onCompositionEnd(
  _state: InputState,
  _canvas: HTMLCanvasElement,
  cb: (device: number, text: string) => void,
): void {
  window.addEventListener("compositionend", (e) => {
    cb(0, (e as CompositionEvent).data);
  });
}
