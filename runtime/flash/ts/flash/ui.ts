/**
 * flash.ui package — ContextMenu, ContextMenuItem, Keyboard, Mouse.
 */

import { EventDispatcher } from "./events";

// ---------------------------------------------------------------------------
// ContextMenuItem
// ---------------------------------------------------------------------------

export class ContextMenuItem extends EventDispatcher {
  caption: string;
  enabled: boolean;
  separatorBefore: boolean;
  visible: boolean;

  constructor(caption = "", separatorBefore = false, enabled = true, visible = true) {
    super();
    this.caption = caption;
    this.separatorBefore = separatorBefore;
    this.enabled = enabled;
    this.visible = visible;
  }

  clone(): ContextMenuItem {
    return new ContextMenuItem(this.caption, this.separatorBefore, this.enabled, this.visible);
  }
}

// ---------------------------------------------------------------------------
// ContextMenu (AS3 NativeMenu on AIR, ContextMenu on web)
// ---------------------------------------------------------------------------

export class ContextMenu extends EventDispatcher {
  customItems: ContextMenuItem[] = [];

  hideBuiltInItems(): void {
    // No built-in items to hide in the shim.
  }

  clone(): ContextMenu {
    const cm = new ContextMenu();
    cm.customItems = this.customItems.map((item) => item.clone());
    return cm;
  }
}

// Alias — AIR uses NativeMenu, web uses ContextMenu. Same shape.
export { ContextMenu as NativeMenu };

// ---------------------------------------------------------------------------
// Keyboard
// ---------------------------------------------------------------------------

export class Keyboard {
  static readonly BACKSPACE = 8;
  static readonly TAB = 9;
  static readonly ENTER = 13;
  static readonly SHIFT = 16;
  static readonly CONTROL = 17;
  static readonly CAPS_LOCK = 20;
  static readonly ESCAPE = 27;
  static readonly SPACE = 32;
  static readonly PAGE_UP = 33;
  static readonly PAGE_DOWN = 34;
  static readonly END = 35;
  static readonly HOME = 36;
  static readonly LEFT = 37;
  static readonly UP = 38;
  static readonly RIGHT = 39;
  static readonly DOWN = 40;
  static readonly INSERT = 45;
  static readonly DELETE = 46;
  static readonly NUMPAD_0 = 96;
  static readonly NUMPAD_1 = 97;
  static readonly NUMPAD_2 = 98;
  static readonly NUMPAD_3 = 99;
  static readonly NUMPAD_4 = 100;
  static readonly NUMPAD_5 = 101;
  static readonly NUMPAD_6 = 102;
  static readonly NUMPAD_7 = 103;
  static readonly NUMPAD_8 = 104;
  static readonly NUMPAD_9 = 105;
  static readonly NUMPAD_MULTIPLY = 106;
  static readonly NUMPAD_ADD = 107;
  static readonly NUMPAD_SUBTRACT = 109;
  static readonly NUMPAD_DECIMAL = 110;
  static readonly NUMPAD_DIVIDE = 111;
  static readonly F1 = 112;
  static readonly F2 = 113;
  static readonly F3 = 114;
  static readonly F4 = 115;
  static readonly F5 = 116;
  static readonly F6 = 117;
  static readonly F7 = 118;
  static readonly F8 = 119;
  static readonly F9 = 120;
  static readonly F10 = 121;
  static readonly F11 = 122;
  static readonly F12 = 123;
  static readonly SEMICOLON = 186;
  static readonly EQUAL = 187;
  static readonly COMMA = 188;
  static readonly MINUS = 189;
  static readonly PERIOD = 190;
  static readonly SLASH = 191;
  static readonly BACKQUOTE = 192;
  static readonly LEFTBRACKET = 219;
  static readonly BACKSLASH = 220;
  static readonly RIGHTBRACKET = 221;
  static readonly QUOTE = 222;

  static readonly A = 65;
  static readonly B = 66;
  static readonly C = 67;
  static readonly D = 68;
  static readonly E = 69;
  static readonly F = 70;
  static readonly G = 71;
  static readonly H = 72;
  static readonly I = 73;
  static readonly J = 74;
  static readonly K = 75;
  static readonly L = 76;
  static readonly M = 77;
  static readonly N = 78;
  static readonly O = 79;
  static readonly P = 80;
  static readonly Q = 81;
  static readonly R = 82;
  static readonly S = 83;
  static readonly T = 84;
  static readonly U = 85;
  static readonly V = 86;
  static readonly W = 87;
  static readonly X = 88;
  static readonly Y = 89;
  static readonly Z = 90;

  static readonly NUMBER_0 = 48;
  static readonly NUMBER_1 = 49;
  static readonly NUMBER_2 = 50;
  static readonly NUMBER_3 = 51;
  static readonly NUMBER_4 = 52;
  static readonly NUMBER_5 = 53;
  static readonly NUMBER_6 = 54;
  static readonly NUMBER_7 = 55;
  static readonly NUMBER_8 = 56;
  static readonly NUMBER_9 = 57;

  static isAccessible(): boolean { return false; }
}

// ---------------------------------------------------------------------------
// Mouse
// ---------------------------------------------------------------------------

export class Mouse {
  static cursor = "auto";
  static supportsCursor = true;
  static supportsNativeCursor = false;

  static hide(): void {
    if (typeof document !== "undefined") {
      document.body.style.cursor = "none";
    }
  }

  static show(): void {
    if (typeof document !== "undefined") {
      document.body.style.cursor = "";
    }
  }

  static registerCursor(_name: string, _data: any): void {}
  static unregisterCursor(_name: string): void {}
}
