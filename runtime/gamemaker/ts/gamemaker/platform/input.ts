/** Browser input — mouse and keyboard event binding. */

import { getCanvas } from "./graphics";

export function onMouseMove(cb: (x: number, y: number) => void): void {
  getCanvas().addEventListener("mousemove", (e) => cb(e.offsetX, e.offsetY));
}

export function onMouseDown(cb: (button: number) => void): void {
  getCanvas().addEventListener("mousedown", (e) => cb(e.button));
  getCanvas().addEventListener("contextmenu", (e) => { e.preventDefault(); e.stopPropagation(); });
}

export function onMouseUp(cb: (button: number) => void): void {
  getCanvas().addEventListener("mouseup", (e) => cb(e.button));
}

export function onKeyDown(cb: (key: string, keyCode: number) => void): void {
  getCanvas().addEventListener("keydown", (e) => cb(e.key, e.keyCode));
}

export function onKeyUp(cb: (key: string, keyCode: number) => void): void {
  getCanvas().addEventListener("keyup", (e) => cb(e.key, e.keyCode));
}
