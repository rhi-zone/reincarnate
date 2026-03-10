/**
 * Flash AS3 global declarations — classes and functions that are in the AVM2
 * global scope and used without an import statement.
 */

/** AS3 global `trace()` — debug logging, equivalent to console.log(). */
declare function trace(...args: any[]): void;

// AS3 Error augmentations:
// - Constructor accepts an optional error ID as second arg
// - Instance has getStackTrace() for introspection
interface ErrorConstructor {
  new(message?: string, id?: number): Error;
  (message?: string, id?: number): Error;
}
interface Error {
  // AS3 Error.getStackTrace() — returns the call stack as a string.
  getStackTrace(): string;
}

/** AS3 `ArgumentError` — thrown when a function receives an invalid argument. */
declare class ArgumentError extends Error {
  constructor(message?: string, id?: number);
}

/** AS3 `IllegalOperationError` — thrown for operations that are not valid. */
declare class IllegalOperationError extends Error {
  constructor(message?: string);
}

/** AS3 `VerifyError` — thrown by the AVM2 verifier on bad bytecode. */
declare class VerifyError extends Error {
  constructor(message?: string);
}
