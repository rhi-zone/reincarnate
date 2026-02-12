/**
 * flash.filters package — BitmapFilter and concrete filter subclasses.
 */

// ---------------------------------------------------------------------------
// BitmapFilter (abstract base)
// ---------------------------------------------------------------------------

export class BitmapFilter {
  clone(): BitmapFilter {
    return new BitmapFilter();
  }
}

// ---------------------------------------------------------------------------
// BlurFilter
// ---------------------------------------------------------------------------

export class BlurFilter extends BitmapFilter {
  blurX: number;
  blurY: number;
  quality: number;

  constructor(blurX = 4, blurY = 4, quality = 1) {
    super();
    this.blurX = blurX;
    this.blurY = blurY;
    this.quality = quality;
  }

  override clone(): BlurFilter {
    return new BlurFilter(this.blurX, this.blurY, this.quality);
  }
}

// ---------------------------------------------------------------------------
// DropShadowFilter
// ---------------------------------------------------------------------------

export class DropShadowFilter extends BitmapFilter {
  distance: number;
  angle: number;
  color: number;
  alpha: number;
  blurX: number;
  blurY: number;
  strength: number;
  quality: number;
  inner: boolean;
  knockout: boolean;
  hideObject: boolean;

  constructor(
    distance = 4,
    angle = 45,
    color = 0x000000,
    alpha = 1,
    blurX = 4,
    blurY = 4,
    strength = 1,
    quality = 1,
    inner = false,
    knockout = false,
    hideObject = false,
  ) {
    super();
    this.distance = distance;
    this.angle = angle;
    this.color = color;
    this.alpha = alpha;
    this.blurX = blurX;
    this.blurY = blurY;
    this.strength = strength;
    this.quality = quality;
    this.inner = inner;
    this.knockout = knockout;
    this.hideObject = hideObject;
  }

  override clone(): DropShadowFilter {
    return new DropShadowFilter(
      this.distance, this.angle, this.color, this.alpha,
      this.blurX, this.blurY, this.strength, this.quality,
      this.inner, this.knockout, this.hideObject,
    );
  }
}

// ---------------------------------------------------------------------------
// GlowFilter
// ---------------------------------------------------------------------------

export class GlowFilter extends BitmapFilter {
  color: number;
  alpha: number;
  blurX: number;
  blurY: number;
  strength: number;
  quality: number;
  inner: boolean;
  knockout: boolean;

  constructor(
    color = 0xff0000,
    alpha = 1,
    blurX = 6,
    blurY = 6,
    strength = 2,
    quality = 1,
    inner = false,
    knockout = false,
  ) {
    super();
    this.color = color;
    this.alpha = alpha;
    this.blurX = blurX;
    this.blurY = blurY;
    this.strength = strength;
    this.quality = quality;
    this.inner = inner;
    this.knockout = knockout;
  }

  override clone(): GlowFilter {
    return new GlowFilter(
      this.color, this.alpha, this.blurX, this.blurY,
      this.strength, this.quality, this.inner, this.knockout,
    );
  }
}

// ---------------------------------------------------------------------------
// ColorMatrixFilter
// ---------------------------------------------------------------------------

export class ColorMatrixFilter extends BitmapFilter {
  matrix: number[];

  constructor(matrix?: number[]) {
    super();
    this.matrix = matrix ?? [
      1, 0, 0, 0, 0,
      0, 1, 0, 0, 0,
      0, 0, 1, 0, 0,
      0, 0, 0, 1, 0,
    ];
  }

  override clone(): ColorMatrixFilter {
    return new ColorMatrixFilter([...this.matrix]);
  }
}

// ---------------------------------------------------------------------------
// BevelFilter
// ---------------------------------------------------------------------------

export class BevelFilter extends BitmapFilter {
  distance: number;
  angle: number;
  highlightColor: number;
  highlightAlpha: number;
  shadowColor: number;
  shadowAlpha: number;
  blurX: number;
  blurY: number;
  strength: number;
  quality: number;
  type: string;
  knockout: boolean;

  constructor(
    distance = 4,
    angle = 45,
    highlightColor = 0xffffff,
    highlightAlpha = 1,
    shadowColor = 0x000000,
    shadowAlpha = 1,
    blurX = 4,
    blurY = 4,
    strength = 1,
    quality = 1,
    type = "inner",
    knockout = false,
  ) {
    super();
    this.distance = distance;
    this.angle = angle;
    this.highlightColor = highlightColor;
    this.highlightAlpha = highlightAlpha;
    this.shadowColor = shadowColor;
    this.shadowAlpha = shadowAlpha;
    this.blurX = blurX;
    this.blurY = blurY;
    this.strength = strength;
    this.quality = quality;
    this.type = type;
    this.knockout = knockout;
  }

  override clone(): BevelFilter {
    return new BevelFilter(
      this.distance, this.angle, this.highlightColor, this.highlightAlpha,
      this.shadowColor, this.shadowAlpha, this.blurX, this.blurY,
      this.strength, this.quality, this.type, this.knockout,
    );
  }
}

// ---------------------------------------------------------------------------
// GradientGlowFilter
// ---------------------------------------------------------------------------

export class GradientGlowFilter extends BitmapFilter {
  distance: number;
  angle: number;
  colors: number[];
  alphas: number[];
  ratios: number[];
  blurX: number;
  blurY: number;
  strength: number;
  quality: number;
  type: string;
  knockout: boolean;

  constructor(
    distance = 4,
    angle = 45,
    colors: number[] = [],
    alphas: number[] = [],
    ratios: number[] = [],
    blurX = 4,
    blurY = 4,
    strength = 1,
    quality = 1,
    type = "inner",
    knockout = false,
  ) {
    super();
    this.distance = distance;
    this.angle = angle;
    this.colors = colors;
    this.alphas = alphas;
    this.ratios = ratios;
    this.blurX = blurX;
    this.blurY = blurY;
    this.strength = strength;
    this.quality = quality;
    this.type = type;
    this.knockout = knockout;
  }

  override clone(): GradientGlowFilter {
    return new GradientGlowFilter(
      this.distance, this.angle, [...this.colors], [...this.alphas],
      [...this.ratios], this.blurX, this.blurY, this.strength,
      this.quality, this.type, this.knockout,
    );
  }
}

// ---------------------------------------------------------------------------
// ConvolutionFilter
// ---------------------------------------------------------------------------

export class ConvolutionFilter extends BitmapFilter {
  matrixX: number;
  matrixY: number;
  matrix: number[];
  divisor: number;
  bias: number;
  preserveAlpha: boolean;
  clamp: boolean;
  color: number;
  alpha: number;

  constructor(
    matrixX = 0,
    matrixY = 0,
    matrix?: number[],
    divisor = 1,
    bias = 0,
    preserveAlpha = true,
    clamp = true,
    color = 0,
    alpha = 0,
  ) {
    super();
    this.matrixX = matrixX;
    this.matrixY = matrixY;
    this.matrix = matrix ?? [];
    this.divisor = divisor;
    this.bias = bias;
    this.preserveAlpha = preserveAlpha;
    this.clamp = clamp;
    this.color = color;
    this.alpha = alpha;
  }

  override clone(): ConvolutionFilter {
    return new ConvolutionFilter(
      this.matrixX, this.matrixY, [...this.matrix], this.divisor,
      this.bias, this.preserveAlpha, this.clamp, this.color, this.alpha,
    );
  }
}
