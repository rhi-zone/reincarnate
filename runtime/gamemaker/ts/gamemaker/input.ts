/** GML input handling — mouse, keyboard. */

import { onMouseMove, onMouseDown, onMouseUp, onKeyDown } from "./platform";
import { roomVariables, sprites, ACTIVE } from "./runtime";

interface ButtonState { pressed: boolean; released: boolean; held: boolean; }

const mouse = {
  x: 0,
  y: 0,
  buttons: [
    { pressed: false, released: false, held: false }, // mb_left (GML 1)
    { pressed: false, released: false, held: false }, // mb_right (GML 2)
    { pressed: false, released: false, held: false }, // mb_middle (GML 3)
  ] as ButtonState[],
};

// DOM button (0=left, 1=middle, 2=right) → mouse.buttons index (0=left, 1=right, 2=middle)
const domButtonMap = [0, 2, 1];

export let keyboard_string = "";

const noop = function () {};

export function mouse_x(): number { return mouse.x; }
export function mouse_y(): number { return mouse.y; }

export function mouse_check_button(button: number): boolean {
  return mouse.buttons[button - 1]?.held ?? false;
}

export function mouse_check_button_pressed(button: number): boolean {
  return mouse.buttons[button - 1]?.pressed ?? false;
}

export function mouse_check_button_released(button: number): boolean {
  return mouse.buttons[button - 1]?.released ?? false;
}

export function resetFrameInput(): void {
  for (const b of mouse.buttons) {
    b.pressed = false;
    b.released = false;
  }
}

export function activateMouse(ax: number, ay: number, override = false): void {
  for (const obj of roomVariables) {
    if (obj.sprite_index === undefined) continue;
    const sprite = sprites[obj.sprite_index];
    if (!sprite) continue;
    const bx = obj.x;
    const by = obj.y;
    const lx = (ax - bx + sprite.origin.x) / obj.image_xscale;
    const ly = (ay - by + sprite.origin.y) / obj.image_yscale;
    if (lx >= 0 && ly >= 0 && lx < sprite.size.width && ly < sprite.size.height) {
      if (override || !obj[ACTIVE]) {
        obj[ACTIVE] = true;
        obj.mouseenter();
      }
    } else {
      if (override || obj[ACTIVE]) {
        obj[ACTIVE] = false;
        obj.mouseleave();
      }
    }
  }
}

export function setupInput(): void {
  onMouseMove((x, y) => {
    mouse.x = x;
    mouse.y = y;
    activateMouse(x, y);
  });

  onMouseDown((button) => {
    const b = mouse.buttons[domButtonMap[button]];
    if (b) { b.pressed = true; b.held = true; }
  });

  onMouseUp((button) => {
    const b = mouse.buttons[domButtonMap[button]];
    if (b) { b.released = true; b.held = false; }
  });

  onKeyDown((key, keyCode) => {
    if (key.length === 1) {
      keyboard_string += key;
      if (keyboard_string.length > 1024) {
        keyboard_string = keyboard_string.slice(keyboard_string.length - 1024);
      }
    } else if (keyCode === 8) {
      if (keyboard_string.length > 0) {
        keyboard_string = keyboard_string.slice(0, -1);
      }
    }
    dispatchKeyPress(keyCode);
  });
}

export function dispatchKeyPress(keyCode: number): void {
  const id = "keypress" + keyCode;
  for (const obj of roomVariables) {
    if ((obj as any)[id] !== noop) {
      (obj as any)[id]();
    }
  }
}
