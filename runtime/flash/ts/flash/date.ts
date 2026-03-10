/**
 * AS3-compatible Date wrapper.
 *
 * AS3's Date class exposes date components as settable properties
 * (e.g. `date.fullYear`, `date.month`, `date.date`), while JavaScript's
 * native Date uses methods (`getFullYear()`, `getMonth()`, `getDate()`).
 * This class extends the native Date and adds AS3-compatible getters/setters
 * so decompiled AS3 code works without modification.
 */
export class Date extends globalThis.Date {
  get fullYear(): number { return this.getFullYear(); }
  set fullYear(v: number) { this.setFullYear(v); }

  get month(): number { return this.getMonth(); }
  set month(v: number) { this.setMonth(v); }

  get date(): number { return this.getDate(); }
  set date(v: number) { this.setDate(v); }

  get hours(): number { return this.getHours(); }
  set hours(v: number) { this.setHours(v); }

  get minutes(): number { return this.getMinutes(); }
  set minutes(v: number) { this.setMinutes(v); }

  get seconds(): number { return this.getSeconds(); }
  set seconds(v: number) { this.setSeconds(v); }

  get milliseconds(): number { return this.getMilliseconds(); }
  set milliseconds(v: number) { this.setMilliseconds(v); }

  get time(): number { return this.getTime(); }
  set time(v: number) { this.setTime(v); }

  get day(): number { return this.getDay(); }

  get fullYearUTC(): number { return this.getUTCFullYear(); }
  set fullYearUTC(v: number) { this.setUTCFullYear(v); }

  get monthUTC(): number { return this.getUTCMonth(); }
  set monthUTC(v: number) { this.setUTCMonth(v); }

  get dateUTC(): number { return this.getUTCDate(); }
  set dateUTC(v: number) { this.setUTCDate(v); }

  get hoursUTC(): number { return this.getUTCHours(); }
  set hoursUTC(v: number) { this.setUTCHours(v); }

  get minutesUTC(): number { return this.getUTCMinutes(); }
  set minutesUTC(v: number) { this.setUTCMinutes(v); }

  get secondsUTC(): number { return this.getUTCSeconds(); }
  set secondsUTC(v: number) { this.setUTCSeconds(v); }

  get millisecondsUTC(): number { return this.getUTCMilliseconds(); }
  set millisecondsUTC(v: number) { this.setUTCMilliseconds(v); }

  get timezoneOffset(): number { return this.getTimezoneOffset(); }
}
