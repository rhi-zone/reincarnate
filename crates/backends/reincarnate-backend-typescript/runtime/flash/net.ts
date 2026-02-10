/**
 * flash.net package — URLRequest, URLLoader, URLLoaderDataFormat,
 * FileReference, SharedObject.
 */

import { EventDispatcher, Event, IOErrorEvent, ProgressEvent, SecurityErrorEvent } from "./events";

// ---------------------------------------------------------------------------
// URLRequest
// ---------------------------------------------------------------------------

export class URLRequest {
  url: string;
  method = "GET";
  data: any = null;
  contentType = "application/x-www-form-urlencoded";
  requestHeaders: any[] = [];
  digest: string | null = null;

  constructor(url = "") {
    this.url = url;
  }
}

// ---------------------------------------------------------------------------
// URLLoaderDataFormat
// ---------------------------------------------------------------------------

export class URLLoaderDataFormat {
  static readonly BINARY = "binary";
  static readonly TEXT = "text";
  static readonly VARIABLES = "variables";
}

// ---------------------------------------------------------------------------
// URLLoader
// ---------------------------------------------------------------------------

export class URLLoader extends EventDispatcher {
  bytesLoaded = 0;
  bytesTotal = 0;
  data: any = null;
  dataFormat = URLLoaderDataFormat.TEXT;

  constructor(request?: URLRequest) {
    super();
    if (request) this.load(request);
  }

  close(): void {}

  load(request: URLRequest): void {
    const url = request.url;
    if (typeof globalThis.fetch !== "function") return;
    globalThis
      .fetch(url, { method: request.method })
      .then((res) => {
        this.bytesTotal = Number(res.headers.get("content-length") ?? 0);
        if (this.dataFormat === URLLoaderDataFormat.BINARY) {
          return res.arrayBuffer().then((buf) => {
            this.data = buf;
            return buf.byteLength;
          });
        }
        return res.text().then((txt) => {
          this.data = txt;
          return txt.length;
        });
      })
      .then((size) => {
        this.bytesLoaded = size as number;
        this.bytesTotal = size as number;
        this.dispatchEvent(new Event(Event.COMPLETE));
      })
      .catch((err) => {
        this.dispatchEvent(
          new IOErrorEvent(IOErrorEvent.IO_ERROR, false, false, String(err)),
        );
      });
  }
}

// ---------------------------------------------------------------------------
// FileReference
// ---------------------------------------------------------------------------

export class FileReference extends EventDispatcher {
  creationDate: Date | null = null;
  creator: string | null = null;
  data: any = null;
  extension: string | null = null;
  modificationDate: Date | null = null;
  name: string | null = null;
  size = 0;
  type: string | null = null;

  browse(_typeFilter?: any[]): boolean {
    return false;
  }

  cancel(): void {}

  download(_request: URLRequest, _defaultFileName?: string): void {}

  load(): void {
    this.dispatchEvent(new Event(Event.COMPLETE));
  }

  save(data: any, defaultFileName?: string): void {
    // Best-effort download via browser API.
    try {
      const blob = new Blob([typeof data === "string" ? data : JSON.stringify(data)]);
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = defaultFileName ?? "download";
      a.click();
      URL.revokeObjectURL(url);
      this.dispatchEvent(new Event(Event.COMPLETE));
    } catch {
      this.dispatchEvent(
        new SecurityErrorEvent(SecurityErrorEvent.SECURITY_ERROR),
      );
    }
  }

  upload(_request: URLRequest, _uploadDataFieldName?: string, _testUpload?: boolean): void {}
}

// ---------------------------------------------------------------------------
// SharedObject
// ---------------------------------------------------------------------------

const SHARED_OBJECT_PREFIX = "flash_so_";

export class SharedObject {
  private _name = "";
  private _data: Record<string, any> = {};
  size = 0;
  client: any = null;
  objectEncoding = 3;

  get data(): Record<string, any> {
    return this._data;
  }

  clear(): void {
    this._data = {};
    this.size = 0;
    if (typeof localStorage !== "undefined") {
      localStorage.removeItem(SHARED_OBJECT_PREFIX + this._name);
    }
  }

  close(): void {}

  flush(_minDiskSpace = 0): string {
    const json = JSON.stringify(this._data);
    this.size = json.length;
    if (typeof localStorage !== "undefined") {
      localStorage.setItem(SHARED_OBJECT_PREFIX + this._name, json);
    }
    return "flushed";
  }

  setDirty(propertyName: string): void {
    void propertyName;
  }

  setProperty(propertyName: string, value: any): void {
    this._data[propertyName] = value;
  }

  static getLocal(name: string, _localPath: string | null = null, _secure = false): SharedObject {
    const so = new SharedObject();
    so._name = name;
    if (typeof localStorage !== "undefined") {
      const stored = localStorage.getItem(SHARED_OBJECT_PREFIX + name);
      if (stored) {
        try {
          so._data = JSON.parse(stored);
          so.size = stored.length;
        } catch {
          so._data = {};
        }
      }
    }
    return so;
  }
}
