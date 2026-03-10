/**
 * Flash AS3 global declarations — classes and functions that are in the AVM2
 * global scope and used without an import statement.
 */

/** AS3 global `trace()` — debug logging, equivalent to console.log(). */
declare function trace(...args: any[]): void;

/** AS3 `ArgumentError` — thrown when a function receives an invalid argument. */
declare class ArgumentError extends Error {
  constructor(message?: string);
}

/** AS3 `IllegalOperationError` — thrown for operations that are not valid. */
declare class IllegalOperationError extends Error {
  constructor(message?: string);
}

/** AS3 `VerifyError` — thrown by the AVM2 verifier on bad bytecode. */
declare class VerifyError extends Error {
  constructor(message?: string);
}
