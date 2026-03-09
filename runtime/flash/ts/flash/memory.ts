/** Flash.Memory — Alchemy / domain memory operations (typed array access). */

const HEAP_SIZE = 1024 * 1024; // 1MB default

/** Per-instance AVM2 domain memory (Alchemy heap). */
export class FlashMemory {
  private readonly _heap: ArrayBuffer;
  private readonly _dv: DataView;

  constructor(heapSize = HEAP_SIZE) {
    this._heap = new ArrayBuffer(heapSize);
    this._dv = new DataView(this._heap);
  }

  load_i8(addr: number): number {
    return this._dv.getInt8(addr);
  }

  load_i16(addr: number): number {
    return this._dv.getInt16(addr, true);
  }

  load_i32(addr: number): number {
    return this._dv.getInt32(addr, true);
  }

  load_f32(addr: number): number {
    return this._dv.getFloat32(addr, true);
  }

  load_f64(addr: number): number {
    return this._dv.getFloat64(addr, true);
  }

  store_i8(addr: number, val: number): void {
    this._dv.setInt8(addr, val);
  }

  store_i16(addr: number, val: number): void {
    this._dv.setInt16(addr, val, true);
  }

  store_i32(addr: number, val: number): void {
    this._dv.setInt32(addr, val, true);
  }

  store_f32(addr: number, val: number): void {
    this._dv.setFloat32(addr, val, true);
  }

  store_f64(addr: number, val: number): void {
    this._dv.setFloat64(addr, val, true);
  }
}

/** Sign-extend 1-bit value to i32. */
export function sxi1(val: number): number {
  return (val & 1) ? -1 : 0;
}

/** Sign-extend 8-bit value to i32. */
export function sxi8(val: number): number {
  return (val << 24) >> 24;
}

/** Sign-extend 16-bit value to i32. */
export function sxi16(val: number): number {
  return (val << 16) >> 16;
}
