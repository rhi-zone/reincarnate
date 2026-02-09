/**
 * flash.geom package — Point, Rectangle, Matrix.
 */

// ---------------------------------------------------------------------------
// Point
// ---------------------------------------------------------------------------

export class Point {
  x: number;
  y: number;

  constructor(x = 0, y = 0) {
    this.x = x;
    this.y = y;
  }

  get length(): number {
    return Math.sqrt(this.x * this.x + this.y * this.y);
  }

  add(v: Point): Point {
    return new Point(this.x + v.x, this.y + v.y);
  }

  clone(): Point {
    return new Point(this.x, this.y);
  }

  copyFrom(sourcePoint: Point): void {
    this.x = sourcePoint.x;
    this.y = sourcePoint.y;
  }

  equals(toCompare: Point): boolean {
    return this.x === toCompare.x && this.y === toCompare.y;
  }

  normalize(thickness: number): void {
    const len = this.length;
    if (len > 0) {
      const scale = thickness / len;
      this.x *= scale;
      this.y *= scale;
    }
  }

  offset(dx: number, dy: number): void {
    this.x += dx;
    this.y += dy;
  }

  setTo(xa: number, ya: number): void {
    this.x = xa;
    this.y = ya;
  }

  subtract(v: Point): Point {
    return new Point(this.x - v.x, this.y - v.y);
  }

  toString(): string {
    return `(x=${this.x}, y=${this.y})`;
  }

  static distance(pt1: Point, pt2: Point): number {
    const dx = pt2.x - pt1.x;
    const dy = pt2.y - pt1.y;
    return Math.sqrt(dx * dx + dy * dy);
  }

  static interpolate(pt1: Point, pt2: Point, f: number): Point {
    return new Point(pt2.x + (pt1.x - pt2.x) * f, pt2.y + (pt1.y - pt2.y) * f);
  }

  static polar(len: number, angle: number): Point {
    return new Point(len * Math.cos(angle), len * Math.sin(angle));
  }
}

// ---------------------------------------------------------------------------
// Rectangle
// ---------------------------------------------------------------------------

export class Rectangle {
  x: number;
  y: number;
  width: number;
  height: number;

  constructor(x = 0, y = 0, width = 0, height = 0) {
    this.x = x;
    this.y = y;
    this.width = width;
    this.height = height;
  }

  get top(): number {
    return this.y;
  }
  set top(value: number) {
    this.height += this.y - value;
    this.y = value;
  }

  get bottom(): number {
    return this.y + this.height;
  }
  set bottom(value: number) {
    this.height = value - this.y;
  }

  get left(): number {
    return this.x;
  }
  set left(value: number) {
    this.width += this.x - value;
    this.x = value;
  }

  get right(): number {
    return this.x + this.width;
  }
  set right(value: number) {
    this.width = value - this.x;
  }

  get topLeft(): Point {
    return new Point(this.x, this.y);
  }
  set topLeft(value: Point) {
    this.width += this.x - value.x;
    this.height += this.y - value.y;
    this.x = value.x;
    this.y = value.y;
  }

  get bottomRight(): Point {
    return new Point(this.right, this.bottom);
  }
  set bottomRight(value: Point) {
    this.width = value.x - this.x;
    this.height = value.y - this.y;
  }

  get size(): Point {
    return new Point(this.width, this.height);
  }
  set size(value: Point) {
    this.width = value.x;
    this.height = value.y;
  }

  clone(): Rectangle {
    return new Rectangle(this.x, this.y, this.width, this.height);
  }

  contains(x: number, y: number): boolean {
    return x >= this.x && x < this.right && y >= this.y && y < this.bottom;
  }

  containsPoint(point: Point): boolean {
    return this.contains(point.x, point.y);
  }

  containsRect(rect: Rectangle): boolean {
    return (
      rect.x >= this.x &&
      rect.y >= this.y &&
      rect.right <= this.right &&
      rect.bottom <= this.bottom
    );
  }

  copyFrom(sourceRect: Rectangle): void {
    this.x = sourceRect.x;
    this.y = sourceRect.y;
    this.width = sourceRect.width;
    this.height = sourceRect.height;
  }

  equals(toCompare: Rectangle): boolean {
    return (
      this.x === toCompare.x &&
      this.y === toCompare.y &&
      this.width === toCompare.width &&
      this.height === toCompare.height
    );
  }

  inflate(dx: number, dy: number): void {
    this.x -= dx;
    this.y -= dy;
    this.width += 2 * dx;
    this.height += 2 * dy;
  }

  inflatePoint(point: Point): void {
    this.inflate(point.x, point.y);
  }

  intersection(toIntersect: Rectangle): Rectangle {
    const x = Math.max(this.x, toIntersect.x);
    const y = Math.max(this.y, toIntersect.y);
    const r = Math.min(this.right, toIntersect.right);
    const b = Math.min(this.bottom, toIntersect.bottom);
    if (r <= x || b <= y) return new Rectangle();
    return new Rectangle(x, y, r - x, b - y);
  }

  intersects(toIntersect: Rectangle): boolean {
    return (
      this.x < toIntersect.right &&
      toIntersect.x < this.right &&
      this.y < toIntersect.bottom &&
      toIntersect.y < this.bottom
    );
  }

  isEmpty(): boolean {
    return this.width <= 0 || this.height <= 0;
  }

  offset(dx: number, dy: number): void {
    this.x += dx;
    this.y += dy;
  }

  offsetPoint(point: Point): void {
    this.offset(point.x, point.y);
  }

  setEmpty(): void {
    this.x = 0;
    this.y = 0;
    this.width = 0;
    this.height = 0;
  }

  setTo(xa: number, ya: number, widthA: number, heightA: number): void {
    this.x = xa;
    this.y = ya;
    this.width = widthA;
    this.height = heightA;
  }

  toString(): string {
    return `(x=${this.x}, y=${this.y}, w=${this.width}, h=${this.height})`;
  }

  union(toUnion: Rectangle): Rectangle {
    if (this.isEmpty()) return toUnion.clone();
    if (toUnion.isEmpty()) return this.clone();
    const x = Math.min(this.x, toUnion.x);
    const y = Math.min(this.y, toUnion.y);
    const r = Math.max(this.right, toUnion.right);
    const b = Math.max(this.bottom, toUnion.bottom);
    return new Rectangle(x, y, r - x, b - y);
  }
}

// ---------------------------------------------------------------------------
// Matrix
// ---------------------------------------------------------------------------

export class Matrix {
  a: number;
  b: number;
  c: number;
  d: number;
  tx: number;
  ty: number;

  constructor(a = 1, b = 0, c = 0, d = 1, tx = 0, ty = 0) {
    this.a = a;
    this.b = b;
    this.c = c;
    this.d = d;
    this.tx = tx;
    this.ty = ty;
  }

  clone(): Matrix {
    return new Matrix(this.a, this.b, this.c, this.d, this.tx, this.ty);
  }

  concat(m: Matrix): void {
    const a = this.a * m.a + this.b * m.c;
    const b = this.a * m.b + this.b * m.d;
    const c = this.c * m.a + this.d * m.c;
    const d = this.c * m.b + this.d * m.d;
    const tx = this.tx * m.a + this.ty * m.c + m.tx;
    const ty = this.tx * m.b + this.ty * m.d + m.ty;
    this.a = a;
    this.b = b;
    this.c = c;
    this.d = d;
    this.tx = tx;
    this.ty = ty;
  }

  createBox(
    scaleX: number,
    scaleY: number,
    rotation = 0,
    tx = 0,
    ty = 0,
  ): void {
    if (rotation !== 0) {
      const cos = Math.cos(rotation);
      const sin = Math.sin(rotation);
      this.a = cos * scaleX;
      this.b = sin * scaleY;
      this.c = -sin * scaleX;
      this.d = cos * scaleY;
    } else {
      this.a = scaleX;
      this.b = 0;
      this.c = 0;
      this.d = scaleY;
    }
    this.tx = tx;
    this.ty = ty;
  }

  createGradientBox(
    width: number,
    height: number,
    rotation = 0,
    tx = 0,
    ty = 0,
  ): void {
    this.createBox(width / 1638.4, height / 1638.4, rotation, tx + width / 2, ty + height / 2);
  }

  deltaTransformPoint(point: Point): Point {
    return new Point(this.a * point.x + this.c * point.y, this.b * point.x + this.d * point.y);
  }

  identity(): void {
    this.a = 1;
    this.b = 0;
    this.c = 0;
    this.d = 1;
    this.tx = 0;
    this.ty = 0;
  }

  invert(): void {
    const det = this.a * this.d - this.b * this.c;
    if (det === 0) return;
    const a = this.d / det;
    const b = -this.b / det;
    const c = -this.c / det;
    const d = this.a / det;
    const tx = (this.c * this.ty - this.d * this.tx) / det;
    const ty = (this.b * this.tx - this.a * this.ty) / det;
    this.a = a;
    this.b = b;
    this.c = c;
    this.d = d;
    this.tx = tx;
    this.ty = ty;
  }

  rotate(angle: number): void {
    const cos = Math.cos(angle);
    const sin = Math.sin(angle);
    const a = this.a * cos - this.b * sin;
    const b = this.a * sin + this.b * cos;
    const c = this.c * cos - this.d * sin;
    const d = this.c * sin + this.d * cos;
    const tx = this.tx * cos - this.ty * sin;
    const ty = this.tx * sin + this.ty * cos;
    this.a = a;
    this.b = b;
    this.c = c;
    this.d = d;
    this.tx = tx;
    this.ty = ty;
  }

  scale(sx: number, sy: number): void {
    this.a *= sx;
    this.b *= sy;
    this.c *= sx;
    this.d *= sy;
    this.tx *= sx;
    this.ty *= sy;
  }

  setTo(
    aa: number,
    ba: number,
    ca: number,
    da: number,
    txa: number,
    tya: number,
  ): void {
    this.a = aa;
    this.b = ba;
    this.c = ca;
    this.d = da;
    this.tx = txa;
    this.ty = tya;
  }

  toString(): string {
    return `(a=${this.a}, b=${this.b}, c=${this.c}, d=${this.d}, tx=${this.tx}, ty=${this.ty})`;
  }

  transformPoint(point: Point): Point {
    return new Point(
      this.a * point.x + this.c * point.y + this.tx,
      this.b * point.x + this.d * point.y + this.ty,
    );
  }

  translate(dx: number, dy: number): void {
    this.tx += dx;
    this.ty += dy;
  }
}
