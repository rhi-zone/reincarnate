/**
 * flash.system package — ApplicationDomain, LoaderContext, Security,
 * Capabilities, System, IME, IMEConversionMode.
 */

import { getDefinitionByName } from "./utils";
import { EventDispatcher } from "./events";

// ---------------------------------------------------------------------------
// ApplicationDomain
// ---------------------------------------------------------------------------

export class ApplicationDomain {
  parentDomain: ApplicationDomain | null;

  private static _current: ApplicationDomain | null = null;

  constructor(parentDomain: ApplicationDomain | null = null) {
    this.parentDomain = parentDomain;
  }

  static get currentDomain(): ApplicationDomain {
    if (!ApplicationDomain._current) {
      ApplicationDomain._current = new ApplicationDomain();
    }
    return ApplicationDomain._current;
  }

  hasDefinition(name: string): boolean {
    try {
      getDefinitionByName(name);
      return true;
    } catch {
      return false;
    }
  }

  getDefinition(name: string): Function {
    return getDefinitionByName(name) as Function;
  }
}

// ---------------------------------------------------------------------------
// LoaderContext
// ---------------------------------------------------------------------------

export class LoaderContext {
  checkPolicyFile: boolean;
  applicationDomain: ApplicationDomain | null;
  // securityDomain is intentionally omitted (no sandbox model in the shim).

  constructor(
    checkPolicyFile = false,
    applicationDomain: ApplicationDomain | null = null,
  ) {
    this.checkPolicyFile = checkPolicyFile;
    this.applicationDomain = applicationDomain;
  }
}

// ---------------------------------------------------------------------------
// Capabilities
// ---------------------------------------------------------------------------

export class Capabilities {
  static readonly avHardwareDisable = true;
  static readonly hasAccessibility = false;
  static readonly hasAudio = true;
  static readonly hasMP3 = true;
  static readonly hasVideoEncoder = false;
  static readonly isDebugger = false;
  static readonly language = "en";
  static readonly localFileReadDisable = true;
  static readonly manufacturer = "Reincarnate";
  static readonly os = typeof navigator !== "undefined" ? navigator.platform : "Unknown";
  static readonly playerType = "PlugIn";
  static readonly screenColor = "color";
  static readonly screenDPI = 72;
  static get screenResolutionX(): number {
    return typeof screen !== "undefined" ? screen.width : 1024;
  }
  static get screenResolutionY(): number {
    return typeof screen !== "undefined" ? screen.height : 768;
  }
  static readonly version = "Reincarnate 1,0,0,0";
}

// ---------------------------------------------------------------------------
// Security
// ---------------------------------------------------------------------------

export class Security {
  static readonly LOCAL_TRUSTED = "localTrusted";
  static readonly LOCAL_WITH_FILE = "localWithFile";
  static readonly LOCAL_WITH_NETWORK = "localWithNetwork";
  static readonly REMOTE = "remote";

  static sandboxType = Security.REMOTE;
  static exactSettings = true;
  static disableAVM1Loading = true;

  static allowDomain(..._domains: string[]): void {}
  static allowInsecureDomain(..._domains: string[]): void {}
  static loadPolicyFile(_url: string): void {}
  static showSettings(_panel?: string): void {}
}

// ---------------------------------------------------------------------------
// IMEConversionMode
// ---------------------------------------------------------------------------

export class IMEConversionMode {
  static readonly ALPHANUMERIC_FULL = "ALPHANUMERIC_FULL";
  static readonly ALPHANUMERIC_HALF = "ALPHANUMERIC_HALF";
  static readonly CHINESE = "CHINESE";
  static readonly JAPANESE_HIRAGANA = "JAPANESE_HIRAGANA";
  static readonly JAPANESE_KATAKANA_FULL = "JAPANESE_KATAKANA_FULL";
  static readonly JAPANESE_KATAKANA_HALF = "JAPANESE_KATAKANA_HALF";
  static readonly KOREAN = "KOREAN";
  static readonly UNKNOWN = "UNKNOWN";
}

// ---------------------------------------------------------------------------
// IME
// ---------------------------------------------------------------------------

export class IME extends EventDispatcher {
  private static _enabled = false;
  private static _conversionMode = IMEConversionMode.UNKNOWN;

  static get enabled(): boolean {
    return IME._enabled;
  }

  static set enabled(value: boolean) {
    IME._enabled = value;
  }

  static get conversionMode(): string {
    return IME._conversionMode;
  }

  static set conversionMode(value: string) {
    IME._conversionMode = value;
  }

  static get isSupported(): boolean {
    return false;
  }

  static setCompositionString(_composition: string): void {}
  static doConversion(): void {}
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

export class System {
  static ime: IME | null = null;
  static totalMemory = 0;

  static get freeMemory(): number {
    return 0;
  }

  static get privateMemory(): number {
    return 0;
  }

  static exit(_code: number): void {}
  static gc(): void {}
  static pause(): void {}
  static resume(): void {}

  static setClipboard(string: string): void {
    if (typeof navigator !== "undefined" && navigator.clipboard) {
      navigator.clipboard.writeText(string).catch(() => {});
    }
  }
}
